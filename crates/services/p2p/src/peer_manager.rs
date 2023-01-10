use crate::config::Config;
use libp2p::{
    core::{
        connection::ConnectionId,
        either::EitherOutput,
    },
    identify::{
        Behaviour as Identify,
        Config as IdentifyConfig,
        Event as IdentifyEvent,
        Info as IdentifyInfo,
    },
    ping::{
        Behaviour as Ping,
        Config as PingConfig,
        Event as PingEvent,
        Success as PingSuccess,
    },
    swarm::{
        derive_prelude::{
            ConnectionClosed,
            ConnectionEstablished,
            DialFailure,
            FromSwarm,
            ListenFailure,
        },
        ConnectionHandler,
        IntoConnectionHandler,
        IntoConnectionHandlerSelect,
        NetworkBehaviour,
        NetworkBehaviourAction,
        PollParameters,
    },
    Multiaddr,
    PeerId,
};

use std::{
    collections::{
        HashMap,
        HashSet,
        VecDeque,
    },
    sync::{
        Arc,
        RwLock,
    },
    task::{
        Context,
        Poll,
    },
    time::Duration,
};
use tokio::time::Interval;
use tracing::debug;

/// Maximum amount of peer's addresses that we are ready to store per peer
const MAX_IDENTIFY_ADDRESSES: usize = 10;
const HEALTH_CHECK_INTERVAL_IN_SECONDS: u64 = 10;

/// Events emitted by PeerInfoBehaviour
#[derive(Debug, Clone)]
pub enum PeerInfoEvent {
    PeerConnected(PeerId),
    PeerDisconnected {
        peer_id: PeerId,
        should_reconnect: bool,
    },
    TooManyPeers {
        peer_to_disconnect: PeerId,
        peer_to_connect: Option<PeerId>,
    },
    ReconnectToPeer(PeerId),
    PeerIdentified {
        peer_id: PeerId,
        addresses: Vec<Multiaddr>,
    },
    PeerInfoUpdated {
        peer_id: PeerId,
    },
}

// `Behaviour` that holds info about peers
pub struct PeerManagerBehaviour {
    ping: Ping,
    identify: Identify,
    peer_manager: PeerManager,
    // regulary checks if reserved nodes are connected
    health_check: Interval,
}

impl PeerManagerBehaviour {
    pub(crate) fn new(
        config: &Config,
        connection_state: Arc<RwLock<ConnectionState>>,
    ) -> Self {
        let identify = {
            let identify_config =
                IdentifyConfig::new("/fuel/1.0".to_string(), config.keypair.public());
            if let Some(interval) = config.identify_interval {
                Identify::new(identify_config.with_interval(interval))
            } else {
                Identify::new(identify_config)
            }
        };

        let ping = {
            let ping_config = PingConfig::new();
            if let Some(interval) = config.info_interval {
                Ping::new(ping_config.with_interval(interval))
            } else {
                Ping::new(ping_config)
            }
        };

        let reserved_peers: HashSet<PeerId> = config
            .reserved_nodes
            .iter()
            .filter_map(PeerId::try_from_multiaddr)
            .collect();

        let peer_manager = PeerManager::new(
            reserved_peers,
            config.max_peers_connected as usize,
            connection_state,
        );

        Self {
            ping,
            identify,
            peer_manager,
            health_check: tokio::time::interval(Duration::from_secs(
                HEALTH_CHECK_INTERVAL_IN_SECONDS,
            )),
        }
    }

    pub fn total_peers_connected(&self) -> usize {
        self.peer_manager.connected_peers.len()
    }

    /// returns an iterator over the connected peers
    pub fn get_peers_ids(&self) -> impl Iterator<Item = &PeerId> {
        self.peer_manager.connected_peers.keys()
    }

    pub fn get_peer_info(&self, peer_id: &PeerId) -> Option<&PeerInfo> {
        self.peer_manager.connected_peers.get(peer_id)
    }

    pub fn insert_peer_addresses(&mut self, peer_id: &PeerId, addresses: Vec<Multiaddr>) {
        self.peer_manager.insert_peer_addresses(peer_id, addresses)
    }
}

impl NetworkBehaviour for PeerManagerBehaviour {
    type ConnectionHandler = IntoConnectionHandlerSelect<
        <Ping as NetworkBehaviour>::ConnectionHandler,
        <Identify as NetworkBehaviour>::ConnectionHandler,
    >;
    type OutEvent = PeerInfoEvent;

    fn new_handler(&mut self) -> Self::ConnectionHandler {
        IntoConnectionHandler::select(
            self.ping.new_handler(),
            self.identify.new_handler(),
        )
    }

    fn addresses_of_peer(&mut self, peer_id: &PeerId) -> Vec<Multiaddr> {
        let mut list = self.ping.addresses_of_peer(peer_id);
        list.extend_from_slice(&self.identify.addresses_of_peer(peer_id));
        list
    }

    fn on_swarm_event(&mut self, event: FromSwarm<Self::ConnectionHandler>) {
        match event {
            FromSwarm::ConnectionEstablished(connection_established) => {
                let ConnectionEstablished {
                    peer_id,
                    other_established,
                    ..
                } = connection_established;

                self.ping.on_swarm_event(FromSwarm::ConnectionEstablished(
                    connection_established,
                ));
                self.identify
                    .on_swarm_event(FromSwarm::ConnectionEstablished(
                        connection_established,
                    ));

                let addresses = self.addresses_of_peer(&peer_id);
                self.insert_peer_addresses(&peer_id, addresses);

                if other_established == 0 {
                    // this is the first connection to a given Peer
                    self.peer_manager.handle_initial_connection(peer_id);
                }
            }
            FromSwarm::ConnectionClosed(connection_closed) => {
                let ConnectionClosed {
                    remaining_established,
                    peer_id,
                    connection_id,
                    endpoint,
                    ..
                } = connection_closed;

                let (ping_handler, identity_handler) =
                    connection_closed.handler.into_inner();

                let ping_event = ConnectionClosed {
                    handler: ping_handler,
                    peer_id,
                    connection_id,
                    endpoint,
                    remaining_established,
                };
                self.ping
                    .on_swarm_event(FromSwarm::ConnectionClosed(ping_event));

                let identify_event = ConnectionClosed {
                    handler: identity_handler,
                    peer_id,
                    connection_id,
                    endpoint,
                    remaining_established,
                };

                self.identify
                    .on_swarm_event(FromSwarm::ConnectionClosed(identify_event));

                if remaining_established == 0 {
                    // this was the last connection to a given Peer
                    self.peer_manager.handle_peer_disconnect(peer_id);
                }
            }
            FromSwarm::AddressChange(e) => {
                self.ping.on_swarm_event(FromSwarm::AddressChange(e));
                self.identify.on_swarm_event(FromSwarm::AddressChange(e));
            }
            FromSwarm::DialFailure(e) => {
                let (ping_handler, identity_handler) = e.handler.into_inner();
                let ping_event = DialFailure {
                    peer_id: e.peer_id,
                    handler: ping_handler,
                    error: e.error,
                };
                let identity_event = DialFailure {
                    peer_id: e.peer_id,
                    handler: identity_handler,
                    error: e.error,
                };
                self.ping.on_swarm_event(FromSwarm::DialFailure(ping_event));
                self.identify
                    .on_swarm_event(FromSwarm::DialFailure(identity_event));
            }
            FromSwarm::ListenFailure(e) => {
                let (ping_handler, identity_handler) = e.handler.into_inner();
                let ping_event = ListenFailure {
                    handler: ping_handler,
                    local_addr: e.local_addr,
                    send_back_addr: e.send_back_addr,
                };
                let identity_event = ListenFailure {
                    handler: identity_handler,
                    local_addr: e.local_addr,
                    send_back_addr: e.send_back_addr,
                };
                self.ping
                    .on_swarm_event(FromSwarm::ListenFailure(ping_event));
                self.identify
                    .on_swarm_event(FromSwarm::ListenFailure(identity_event));
            }
            FromSwarm::NewListener(e) => {
                self.ping.on_swarm_event(FromSwarm::NewListener(e));
                self.identify.on_swarm_event(FromSwarm::NewListener(e));
            }
            FromSwarm::ExpiredListenAddr(e) => {
                self.ping.on_swarm_event(FromSwarm::ExpiredListenAddr(e));
                self.identify
                    .on_swarm_event(FromSwarm::ExpiredListenAddr(e));
            }
            FromSwarm::ListenerError(e) => {
                self.ping.on_swarm_event(FromSwarm::ListenerError(e));
                self.identify.on_swarm_event(FromSwarm::ListenerError(e));
            }
            FromSwarm::ListenerClosed(e) => {
                self.ping.on_swarm_event(FromSwarm::ListenerClosed(e));
                self.identify.on_swarm_event(FromSwarm::ListenerClosed(e));
            }
            FromSwarm::NewExternalAddr(e) => {
                self.ping.on_swarm_event(FromSwarm::NewExternalAddr(e));
                self.identify.on_swarm_event(FromSwarm::NewExternalAddr(e));
            }
            FromSwarm::ExpiredExternalAddr(e) => {
                self.ping.on_swarm_event(FromSwarm::ExpiredExternalAddr(e));
                self.identify
                    .on_swarm_event(FromSwarm::ExpiredExternalAddr(e));
            }
            FromSwarm::NewListenAddr(e) => {
                self.ping.on_swarm_event(FromSwarm::NewListenAddr(e));
                self.identify.on_swarm_event(FromSwarm::NewListenAddr(e));
            }
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<Self::OutEvent, Self::ConnectionHandler>> {
        if self.health_check.poll_tick(cx).is_ready() {
            let disconnected_peers: Vec<_> = self
                .peer_manager
                .get_disconnected_reserved_peers()
                .copied()
                .collect();

            for peer_id in disconnected_peers {
                debug!(target: "fuel-libp2p", "Trying to reconnect to reserved peer {:?}", peer_id);

                self.peer_manager
                    .pending_events
                    .push_back(PeerInfoEvent::ReconnectToPeer(peer_id));
            }
        }

        if let Some(event) = self.peer_manager.pending_events.pop_front() {
            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event))
        }

        loop {
            match self.ping.poll(cx, params) {
                Poll::Pending => break,
                Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                    peer_id,
                    handler,
                    event,
                }) => {
                    return Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                        peer_id,
                        handler,
                        event: EitherOutput::First(event),
                    })
                }
                Poll::Ready(NetworkBehaviourAction::ReportObservedAddr {
                    address,
                    score,
                }) => {
                    return Poll::Ready(NetworkBehaviourAction::ReportObservedAddr {
                        address,
                        score,
                    })
                }
                Poll::Ready(NetworkBehaviourAction::CloseConnection {
                    peer_id,
                    connection,
                }) => {
                    return Poll::Ready(NetworkBehaviourAction::CloseConnection {
                        peer_id,
                        connection,
                    })
                }
                Poll::Ready(NetworkBehaviourAction::Dial { handler, opts }) => {
                    let handler = IntoConnectionHandler::select(
                        handler,
                        self.identify.new_handler(),
                    );

                    return Poll::Ready(NetworkBehaviourAction::Dial { handler, opts })
                }
                Poll::Ready(NetworkBehaviourAction::GenerateEvent(PingEvent {
                    peer,
                    result: Ok(PingSuccess::Ping { rtt }),
                })) => {
                    self.peer_manager.insert_latest_ping(&peer, rtt);
                    let event = PeerInfoEvent::PeerInfoUpdated { peer_id: peer };
                    return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event))
                }
                _ => {}
            }
        }

        loop {
            match self.identify.poll(cx, params) {
                Poll::Pending => break,
                Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                    peer_id,
                    handler,
                    event,
                }) => {
                    return Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                        peer_id,
                        handler,
                        event: EitherOutput::Second(event),
                    })
                }
                Poll::Ready(NetworkBehaviourAction::ReportObservedAddr {
                    address,
                    score,
                }) => {
                    return Poll::Ready(NetworkBehaviourAction::ReportObservedAddr {
                        address,
                        score,
                    })
                }
                Poll::Ready(NetworkBehaviourAction::CloseConnection {
                    peer_id,
                    connection,
                }) => {
                    return Poll::Ready(NetworkBehaviourAction::CloseConnection {
                        peer_id,
                        connection,
                    })
                }
                Poll::Ready(NetworkBehaviourAction::Dial { handler, opts }) => {
                    let handler =
                        IntoConnectionHandler::select(self.ping.new_handler(), handler);
                    return Poll::Ready(NetworkBehaviourAction::Dial { handler, opts })
                }
                Poll::Ready(NetworkBehaviourAction::GenerateEvent(event)) => {
                    match event {
                        IdentifyEvent::Received {
                            peer_id,
                            info:
                                IdentifyInfo {
                                    protocol_version,
                                    agent_version,
                                    mut listen_addrs,
                                    ..
                                },
                        } => {
                            if listen_addrs.len() > MAX_IDENTIFY_ADDRESSES {
                                debug!(
                                    target: "fuel-libp2p",
                                    "Node {:?} has reported more than {} addresses; it is identified by {:?} and {:?}",
                                    peer_id, MAX_IDENTIFY_ADDRESSES, protocol_version, agent_version
                                );
                                listen_addrs.truncate(MAX_IDENTIFY_ADDRESSES);
                            }

                            self.peer_manager
                                .insert_client_version(&peer_id, agent_version);

                            self.peer_manager
                                .insert_peer_addresses(&peer_id, listen_addrs.clone());

                            let event = PeerInfoEvent::PeerIdentified {
                                peer_id,
                                addresses: listen_addrs,
                            };
                            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(
                                event,
                            ))
                        }
                        IdentifyEvent::Error { peer_id, error } => {
                            debug!(target: "fuel-libp2p", "Identification with peer {:?} failed => {}", peer_id, error)
                        }
                        _ => {}
                    }
                }
            }
        }

        Poll::Pending
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: <<Self::ConnectionHandler as IntoConnectionHandler>::Handler as
            ConnectionHandler>::OutEvent,
    ) {
        match event {
            EitherOutput::First(ping_event) => {
                self.ping
                    .on_connection_handler_event(peer_id, connection_id, ping_event)
            }
            EitherOutput::Second(identify_event) => self
                .identify
                .on_connection_handler_event(peer_id, connection_id, identify_event),
        }
    }
}

// Info about a single Peer that we're connected to
#[derive(Debug, Default, Clone)]
pub struct PeerInfo {
    pub peer_addresses: HashSet<Multiaddr>,
    pub client_version: Option<String>,
    pub latest_ping: Option<Duration>,
}

/// Manages Peers and their events
#[derive(Debug, Default, Clone)]
struct PeerManager {
    pending_events: VecDeque<PeerInfoEvent>,
    connected_peers: HashMap<PeerId, PeerInfo>,
    reserved_peers: HashSet<PeerId>,
    number_of_non_reserved_peers_allowed: usize,
    connection_state: Arc<RwLock<ConnectionState>>,
}

impl PeerManager {
    fn new(
        reserved_peers: HashSet<PeerId>,
        max_connections_allowed: usize,
        connection_state: Arc<RwLock<ConnectionState>>,
    ) -> Self {
        Self {
            pending_events: VecDeque::default(),
            connected_peers: HashMap::with_capacity(max_connections_allowed),
            reserved_peers,
            number_of_non_reserved_peers_allowed: max_connections_allowed,
            connection_state,
        }
    }

    fn insert_peer_addresses(&mut self, peer_id: &PeerId, addresses: Vec<Multiaddr>) {
        if let Some(peer) = self.connected_peers.get_mut(peer_id) {
            for address in addresses {
                peer.peer_addresses.insert(address);
            }
        } else {
            log_missing_peer(peer_id);
        }
    }

    fn insert_latest_ping(&mut self, peer_id: &PeerId, duration: Duration) {
        if let Some(peer) = self.connected_peers.get_mut(peer_id) {
            peer.latest_ping = Some(duration);
        } else {
            log_missing_peer(peer_id);
        }
    }

    fn insert_client_version(&mut self, peer_id: &PeerId, client_version: String) {
        if let Some(peer) = self.connected_peers.get_mut(peer_id) {
            peer.client_version = Some(client_version);
        } else {
            log_missing_peer(peer_id);
        }
    }

    fn get_disconnected_reserved_peers(&self) -> impl Iterator<Item = &PeerId> {
        self.reserved_peers
            .iter()
            .filter(|peer_id| !self.connected_peers.contains_key(peer_id))
    }

    fn reserved_peers_connected_count(&self) -> usize {
        self.reserved_peers.iter().fold(0, |count, peer_id| {
            if self.connected_peers.contains_key(peer_id) {
                count + 1
            } else {
                count
            }
        })
    }

    fn find_disconnected_reserved_peer(&self) -> Option<PeerId> {
        self.reserved_peers
            .iter()
            .find(|peer_id| self.connected_peers.contains_key(peer_id))
            .cloned()
    }

    /// Handles the first connnection established with a Peer
    fn handle_initial_connection(&mut self, peer_id: PeerId) {
        // if the connected Peer is not from the reserved peers
        if !self.reserved_peers.contains(&peer_id) {
            let number_of_non_reserved_peers_connected =
                self.connected_peers.len() - self.reserved_peers_connected_count();

            // check if there is no more space for non-resereved peers
            if number_of_non_reserved_peers_connected
                >= self.number_of_non_reserved_peers_allowed
            {
                // todo/potential improvement: once `Peer Reputation` is implemented we could check if there are peers
                // with poor reputation and disconnect them instead?

                // Too many peers already connected, disconnect the Peer
                self.pending_events.push_back(PeerInfoEvent::TooManyPeers {
                    peer_to_disconnect: peer_id,
                    peer_to_connect: self.find_disconnected_reserved_peer(),
                });

                // early exit, we don't want to report new peer connection
                // nor insert it in `connected_peers`
                // since we're going to disconnect it anyways
                return
            } else if self.number_of_non_reserved_peers_allowed
                - number_of_non_reserved_peers_connected
                == 1
            {
                // this is the last peer allowed no more!
                self.connection_state
                    .write()
                    .unwrap()
                    .non_reserved_peers_allowed = false;
            }
        }

        // insert and report on new Peer Connection
        // for either, reserved peer or non-reserved
        self.connected_peers.insert(peer_id, PeerInfo::default());
        self.pending_events
            .push_back(PeerInfoEvent::PeerConnected(peer_id));
    }

    /// Handles on peer's last connection getting disconnected
    fn handle_peer_disconnect(&mut self, peer_id: PeerId) {
        // try immediate reconnect if it's a reserved peer
        let should_reconnect = self.reserved_peers.contains(&peer_id);
        self.connected_peers.remove(&peer_id);

        if !should_reconnect
            && !self
                .connection_state
                .read()
                .unwrap()
                .non_reserved_peers_allowed
        {
            self.connection_state
                .write()
                .unwrap()
                .non_reserved_peers_allowed = true;
        }

        self.pending_events
            .push_back(PeerInfoEvent::PeerDisconnected {
                peer_id,
                should_reconnect,
            })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ConnectionState {
    non_reserved_peers_allowed: bool,
}

impl ConnectionState {
    pub fn new() -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(Self {
            non_reserved_peers_allowed: true,
        }))
    }

    pub fn available_slot(&self) -> bool {
        self.non_reserved_peers_allowed
    }
}

fn log_missing_peer(peer_id: &PeerId) {
    debug!(target: "fuel-libp2p", "Peer with PeerId: {:?} is not among the connected peers", peer_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_random_peers(size: usize) -> Vec<PeerId> {
        (0..size).map(|_| PeerId::random()).collect()
    }

    #[test]
    fn test_peer_manager_struct() {
        let reserved_peer_size = 5;
        let max_non_reserved_allowed = 15;
        let total_connections = max_non_reserved_allowed + reserved_peer_size;
        let reserved_peers = get_random_peers(reserved_peer_size);
        let random_peers = get_random_peers(max_non_reserved_allowed * 2);

        let connection_state = ConnectionState::new();

        let mut peer_manager = PeerManager::new(
            reserved_peers.clone().into_iter().collect(),
            max_non_reserved_allowed,
            connection_state,
        );

        // try connecting only random peers
        for peer_id in &random_peers {
            peer_manager.handle_initial_connection(*peer_id);
        }

        // only amount of non-reserved peers allowed should be connected
        assert_eq!(
            peer_manager.connected_peers.len(),
            peer_manager.number_of_non_reserved_peers_allowed
        );
        // or in other words:
        assert_eq!(peer_manager.connected_peers.len(), random_peers.len() / 2);

        // connect resereved peers
        for peer_id in &reserved_peers {
            peer_manager.handle_initial_connection(*peer_id);
        }

        // the connections should be at max now
        assert_eq!(peer_manager.connected_peers.len(), total_connections);

        // disconnect a reserved peer
        peer_manager.handle_peer_disconnect(*reserved_peers.first().unwrap());
        assert_eq!(peer_manager.connected_peers.len(), total_connections - 1);

        // assert that the last random peer is not already connected
        assert!(!peer_manager
            .connected_peers
            .contains_key(random_peers.last().unwrap()));

        // try to connect the last random peer in the list
        peer_manager.handle_initial_connection(*random_peers.last().unwrap());

        // the connection count should remain the same as when the reserved peer disconnected
        // that is, the connection has been refused
        assert_eq!(peer_manager.connected_peers.len(), total_connections - 1);

        // reconnect the first reserved peer that was disconnected
        peer_manager.handle_initial_connection(*reserved_peers.first().unwrap());
        assert_eq!(peer_manager.connected_peers.len(), total_connections);

        // disconnect a single non-reserved peer
        peer_manager.handle_peer_disconnect(*random_peers.first().unwrap());
        assert_eq!(peer_manager.connected_peers.len(), total_connections - 1);

        // connect a different non-reserved peer
        peer_manager.handle_initial_connection(*random_peers.last().unwrap());

        // the connection should be successful,
        // and we should be up to our max connections count again
        assert_eq!(peer_manager.connected_peers.len(), total_connections);
    }
}
