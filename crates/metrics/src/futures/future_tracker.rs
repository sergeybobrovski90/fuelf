use crate::futures::FuturesMetrics;
use core::{
    future::Future,
    pin::Pin,
    task::{
        Context,
        Poll,
    },
    time::Duration,
};
use std::time::Instant;

/// The execution report of the future, generated by [`FutureTracker`].
#[derive(Debug)]
pub struct ExecutionTime<Output> {
    /// The time spent for real action of the future.
    busy: Duration,
    /// The idle time of the future.
    idle: Duration,
    /// The output of the future.
    output: Output,
}

impl<Output> ExecutionTime<Output> {
    /// Extracts the future output and records the execution report into the metrics.
    pub fn extract(self, metric: &FuturesMetrics) -> Output {
        // TODO: Use `u128` when `AtomicU128` is stable.
        metric.busy.inc_by(
            u64::try_from(self.busy.as_nanos())
                .expect("The task doesn't live longer than `u64`"),
        );
        metric.idle.inc_by(
            u64::try_from(self.idle.as_nanos())
                .expect("The task doesn't live longer than `u64`"),
        );
        self.output
    }
}

/// A guard representing a span which has been entered and is currently
/// executing.
///
/// When the guard is dropped, the span will be exited.
///
/// This is returned by the [`Span::enter`] function.
#[derive(Debug)]
#[must_use = "once a span has been entered, it should be exited"]
struct Entered<'a> {
    span: &'a mut Span,
    busy_instant: Instant,
}

impl<'a> Entered<'a> {
    fn new(span: &'a mut Span) -> Self {
        Self {
            span,
            busy_instant: Instant::now(),
        }
    }
}

impl<'a> Drop for Entered<'a> {
    #[inline(always)]
    fn drop(&mut self) {
        self.span.busy = self.span.busy.saturating_add(self.busy_instant.elapsed());
        self.span.do_exit()
    }
}

/// A handle representing a span, with the capability to enter the span if it
/// exists.
#[derive(Default, Debug, Clone)]
struct Span {
    /// The cumulative busy(active) time of the future across all `poll` calls.
    busy: Duration,
    /// The cumulative idle time of the future across all `poll` calls.
    idle: Duration,
    /// An [`Instant`] to track the idle time.
    ///
    /// If this is `None`, then the span has either closed or was never enabled.
    idle_instant: Option<Instant>,
}

impl Span {
    /// Enters this span, returning a guard that will exit the span when dropped.
    #[inline(always)]
    pub fn enter(&mut self) -> Entered<'_> {
        self.do_enter();
        Entered::new(self)
    }

    #[inline(always)]
    fn do_enter(&mut self) {
        let idle_instant = core::mem::take(&mut self.idle_instant);

        if let Some(idle_instant) = idle_instant {
            self.idle = self.idle.saturating_add(idle_instant.elapsed());
        }
    }

    #[inline(always)]
    fn do_exit(&mut self) {
        self.idle_instant = Some(Instant::now());
    }
}

pin_project_lite::pin_project! {
    /// A [`Future`] that has been tracked with a [`Span`].
    /// It tracks the execution time of the future, it's active and idle phases.
    #[derive(Debug, Clone)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct FutureTracker<T> {
        #[pin]
        inner: T,
        span: Span,
    }
}

impl<T> FutureTracker<T> {
    /// Creates a [`FutureTracker`] wrapper around the `inner` future.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            span: Default::default(),
        }
    }
}

impl<T: Future> Future for FutureTracker<T> {
    type Output = ExecutionTime<T::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let enter = this.span.enter();
        let output = this.inner.poll(cx);

        match output {
            Poll::Ready(output) => {
                drop(enter);
                Poll::Ready(ExecutionTime {
                    busy: this.span.busy,
                    idle: this.span.idle,
                    output,
                })
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_future() {
        let future = async { tokio::task::yield_now().await };
        let wrapper_future = FutureTracker::new(future);
        let result = wrapper_future.await;
        assert_eq!(result.idle.as_secs(), 0)
    }

    #[tokio::test]
    async fn idle_time_correct() {
        let future = async { tokio::time::sleep(Duration::from_secs(1)).await };
        let wrapper_future = FutureTracker::new(future);
        let result = wrapper_future.await;
        assert_eq!(result.idle.as_secs(), 1)
    }

    #[tokio::test]
    async fn busy_time_correct() {
        let future = async { std::thread::sleep(Duration::from_secs(1)) };
        let wrapper_future = FutureTracker::new(future);
        let result = wrapper_future.await;
        assert_eq!(result.idle.as_secs(), 0);
        assert_eq!(result.busy.as_secs(), 1);
    }

    #[tokio::test]
    async fn hybrid_time_correct() {
        let future = async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            std::thread::sleep(Duration::from_secs(1));
        };
        let wrapper_future = FutureTracker::new(future);
        let result = wrapper_future.await;
        assert_eq!(result.idle.as_secs(), 2);
        assert_eq!(result.busy.as_secs(), 1);
    }

    #[tokio::test]
    async fn hybrid_time_correct_complex_case() {
        let future = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            std::thread::sleep(Duration::from_secs(1));
            tokio::time::sleep(Duration::from_secs(2)).await;
            std::thread::sleep(Duration::from_secs(2));
            tokio::time::sleep(Duration::from_secs(3)).await;
        };
        let wrapper_future = FutureTracker::new(future);
        let result = wrapper_future.await;
        assert_eq!(result.idle.as_secs(), 6);
        assert_eq!(result.busy.as_secs(), 3);
    }
}
