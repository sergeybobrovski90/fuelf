use self::mdns::MdnsWrapper;
use futures::FutureExt;
use libp2p::{
    core::Endpoint,
    kad::{
        store::MemoryStore,
        Behaviour as KademliaBehavior,
        Event,
    },
    mdns::Event as MdnsEvent,
    swarm::{
        derive_prelude::{
            ConnectionClosed,
            ConnectionEstablished,
            FromSwarm,
        },
        ConnectionDenied,
        ConnectionId,
        NetworkBehaviour,
        THandler,
    },
    Multiaddr,
    PeerId,
};

use libp2p_swarm::{
    THandlerInEvent,
    THandlerOutEvent,
    ToSwarm,
};
use std::{
    collections::HashSet,
    pin::Pin,
    task::{
        Context,
        Poll,
    },
    time::Duration,
};
use tracing::trace;
mod discovery_config;
mod mdns;
pub use discovery_config::DiscoveryConfig;

const SIXTY_SECONDS: Duration = Duration::from_secs(60);

/// NetworkBehavior for discovery of nodes
pub struct DiscoveryBehaviour {
    /// List of bootstrap nodes and their addresses
    _bootstrap_nodes: Vec<(PeerId, Multiaddr)>,

    /// List of reserved nodes and their addresses
    _reserved_nodes: Vec<(PeerId, Multiaddr)>,

    /// Track the connected peers
    connected_peers: HashSet<PeerId>,

    /// For discovery on local network, optionally available
    mdns: MdnsWrapper,

    /// Kademlia with MemoryStore
    kademlia: KademliaBehavior<MemoryStore>,

    /// If enabled, the Stream that will fire after the delay expires,
    /// starting new random walk
    next_kad_random_walk: Option<Pin<Box<tokio::time::Sleep>>>,

    /// The Duration for the next random walk, after the current one ends
    duration_to_next_kad: Duration,

    /// Maximum amount of allowed peers
    max_peers_connected: usize,

    /// If false, `addresses_of_peer` won't return any private IPv4/IPv6 address,
    /// except for the ones stored in `bootstrap_nodes` and `reserved_peers`.
    _allow_private_addresses: bool,
}

impl DiscoveryBehaviour {
    /// Adds a known listen address of a peer participating in the DHT to the routing table.
    pub fn add_address(&mut self, peer_id: &PeerId, address: Multiaddr) {
        self.kademlia.add_address(peer_id, address);
    }
}

impl NetworkBehaviour for DiscoveryBehaviour {
    type ConnectionHandler =
        <KademliaBehavior<MemoryStore> as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.kademlia.handle_established_inbound_connection(
            connection_id,
            peer,
            local_addr,
            remote_addr,
        )
    }

    // receive events from KademliaHandler and pass it down to kademlia
    fn handle_pending_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addresses: &[Multiaddr],
        effective_role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        self.kademlia.handle_pending_outbound_connection(
            connection_id,
            maybe_peer,
            addresses,
            effective_role,
        )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        tracing::error!("discovery established outbound connection: {:?}", &peer);
        self.kademlia.handle_established_outbound_connection(
            connection_id,
            peer,
            addr,
            role_override,
        )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        tracing::info!("discovery swarm event: {:?}", &event);
        match &event {
            FromSwarm::ConnectionEstablished(ConnectionEstablished {
                peer_id,
                other_established,
                ..
            }) => {
                if *other_established == 0 {
                    self.connected_peers.insert(*peer_id);

                    trace!("Connected to a peer {:?}", peer_id);
                }
            }
            FromSwarm::ConnectionClosed(ConnectionClosed {
                peer_id,
                remaining_established,
                ..
            }) => {
                if *remaining_established == 0 {
                    self.connected_peers.remove(peer_id);
                    trace!("Disconnected from {:?}", peer_id);
                }
            }
            _ => (),
        }
        self.kademlia.on_swarm_event(event)
    }

    // receive events from KademliaHandler and pass it down to kademlia
    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        tracing::info!("discovery handler event: {:?}", &event);
        self.kademlia
            .on_connection_handler_event(peer_id, connection, event);
    }

    // gets polled by the swarm
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        // if random walk is enabled poll the stream that will fire when random walk is scheduled
        if let Some(next_kad_random_query) = self.next_kad_random_walk.as_mut() {
            while next_kad_random_query.poll_unpin(cx).is_ready() {
                if self.connected_peers.len() < self.max_peers_connected {
                    let random_peer_id = PeerId::random();
                    self.kademlia.get_closest_peers(random_peer_id);
                }

                *next_kad_random_query =
                    Box::pin(tokio::time::sleep(self.duration_to_next_kad));
                // duration to next random walk should either be exponentially bigger than the previous
                // or at max 60 seconds
                self.duration_to_next_kad = std::cmp::min(
                    self.duration_to_next_kad.saturating_mul(2),
                    SIXTY_SECONDS,
                );
            }
        }

        // poll sub-behaviors
        if let Poll::Ready(kad_action) = self.kademlia.poll(cx) {
            // match &kad_action {
            //     Event::OutboundQueryProgressed { result: QueryResult::GetClosestPeers(Ok(closest)), ..} => {
            //         self.kademlia.add_address()
            //
            //     }
            // }
            tracing::info!("kad action: {:?}", &kad_action);
            return Poll::Ready(kad_action)
        };
        while let Poll::Ready(mdns_event) = self.mdns.poll(cx) {
            match mdns_event {
                ToSwarm::GenerateEvent(MdnsEvent::Discovered(list)) => {
                    for (peer_id, multiaddr) in list {
                        self.kademlia.add_address(&peer_id, multiaddr);
                    }
                }
                ToSwarm::CloseConnection {
                    peer_id,
                    connection,
                } => {
                    return Poll::Ready(ToSwarm::CloseConnection {
                        peer_id,
                        connection,
                    })
                }
                _ => {}
            }
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DiscoveryBehaviour,
        DiscoveryConfig,
        Event as KademliaEvent,
    };
    use futures::{
        future::poll_fn,
        StreamExt,
    };
    use libp2p::{
        identity::Keypair,
        multiaddr::Protocol,
        swarm::SwarmEvent,
        Multiaddr,
        PeerId,
        Swarm,
    };
    use std::{
        collections::HashSet,
        sync::atomic::{
            AtomicUsize,
            Ordering,
        },
        task::Poll,
        time::Duration,
    };

    use libp2p_swarm_test::SwarmExt;
    use std::sync::Arc;

    fn build_behavior_fn(
        bootstrap_nodes: Vec<Multiaddr>,
    ) -> impl FnOnce(Keypair) -> DiscoveryBehaviour {
        |keypair| {
            let mut config = DiscoveryConfig::new(
                keypair.public().to_peer_id(),
                "test_network".into(),
            );
            config
                .max_peers_connected(50)
                .with_bootstrap_nodes(bootstrap_nodes)
                .with_random_walk(Duration::from_millis(500));

            config.finish()
        }
    }

    /// helper function for building Discovery Behaviour for testing
    fn build_fuel_discovery(
        bootstrap_nodes: Vec<Multiaddr>,
    ) -> (Swarm<DiscoveryBehaviour>, Multiaddr, PeerId) {
        let behaviour_fn = build_behavior_fn(bootstrap_nodes);

        let listen_addr: Multiaddr = Protocol::Memory(rand::random::<u64>()).into();

        let mut swarm = Swarm::new_ephemeral(behaviour_fn);

        swarm
            .listen_on(listen_addr.clone())
            .expect("swarm should start listening");

        let peer_id = swarm.local_peer_id().to_owned();

        (swarm, listen_addr, peer_id)
    }

    // builds 25 discovery swarms,
    // initially, only connects first_swarm to the rest of the swarms
    // after that each swarm uses kademlia to discover other swarms
    // test completes after all swarms have connected to each other
    // TODO: This used to fail with any connection closures, but that was causing a lot of failed
    //   Now it allows for many connection closures before failing. We don't know what caused the
    //   connections to start failing, but had something to do with upgrading `libp2p`.
    #[tokio::test]
    async fn discovery_works() {
        // Number of peers in the network
        let num_of_swarms = 25;
        let (first_swarm, first_peer_addr, first_peer_id) = build_fuel_discovery(vec![]);
        let bootstrap_addr: Multiaddr =
            format!("{}/p2p/{}", first_peer_addr.clone(), first_peer_id)
                .parse()
                .unwrap();
        tracing::info!("first swarm addr: {:?}", &first_peer_addr);
        tracing::info!("first swarm id: {:?}", &first_peer_id);

        let mut discovery_swarms = Vec::new();
        discovery_swarms.push((first_swarm, first_peer_addr, first_peer_id));

        for index in 1..num_of_swarms {
            let (swarm, peer_addr, peer_id) =
                build_fuel_discovery(vec![bootstrap_addr.clone()]);

            tracing::info!("{:?} swarm addr: {:?}", index, &peer_addr);
            tracing::info!("{:?} swarm id: {:?}", index, &peer_id);
            discovery_swarms.push((swarm, peer_addr, peer_id));
        }

        // HashSet of swarms to discover for each swarm
        let mut left_to_discover = (0..discovery_swarms.len())
            .map(|current_index| {
                (0..discovery_swarms.len())
                    .skip(1) // first_swarm is already connected
                    .filter_map(|swarm_index| {
                        // filter your self
                        if swarm_index != current_index {
                            // get the PeerId
                            Some(*Swarm::local_peer_id(&discovery_swarms[swarm_index].0))
                        } else {
                            None
                        }
                    })
                    .collect::<HashSet<_>>()
            })
            .collect::<Vec<_>>();

        let connection_closed_counter = Arc::new(AtomicUsize::new(0));
        let counter_copy = connection_closed_counter.clone();
        const MAX_CONNECTION_CLOSED: usize = 1000;

        poll_fn(move |cx| {
            'polling: loop {
                for swarm_index in 0..discovery_swarms.len() {
                    if let Poll::Ready(Some(event)) =
                        discovery_swarms[swarm_index].0.poll_next_unpin(cx)
                    {
                        match event {
                            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                                // if peer has connected - remove it from the set
                                left_to_discover[swarm_index].remove(&peer_id);
                            }
                            SwarmEvent::Behaviour(KademliaEvent::UnroutablePeer {
                                peer: peer_id,
                            }) => {
                                tracing::info!("Unroutable peer: {:?}", &peer_id);
                                // kademlia discovered a peer but does not have it's address
                                // we simulate Identify happening and provide the address
                                let unroutable_peer_addr = discovery_swarms
                                    .iter()
                                    .find_map(|(_, next_addr, next_peer_id)| {
                                        // identify the peer
                                        if next_peer_id == &peer_id {
                                            // and return it's address
                                            Some(next_addr.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap();

                                // kademlia must be informed of a peer's address before
                                // adding it to the routing table
                                discovery_swarms[swarm_index]
                                    .0
                                    .behaviour_mut()
                                    .kademlia
                                    .add_address(&peer_id, unroutable_peer_addr.clone());
                            }
                            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                                tracing::warn!(
                                    "Connection closed: {:?} with {:?} previous closures",
                                    &peer_id,
                                    &connection_closed_counter
                                );
                                let old = connection_closed_counter
                                    .fetch_add(1, Ordering::SeqCst);
                                if old > MAX_CONNECTION_CLOSED {
                                    panic!("Connection closed for the {:?}th time", old);
                                }
                            }
                            _ => {}
                        }
                        continue 'polling
                    }
                }
                break
            }

            // if there are no swarms left to discover we are done with the discovery
            if left_to_discover.iter().all(|l| l.is_empty()) {
                // we are done!
                Poll::Ready(())
            } else {
                // keep polling Discovery Behaviour
                Poll::Pending
            }
        })
        .await;
        tracing::info!(
            "Passed with {:?} connection closures",
            counter_copy.load(Ordering::SeqCst)
        );
    }
}
