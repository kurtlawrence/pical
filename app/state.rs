use std::future::Future;

use tokio::sync::{
    mpsc::{channel, Receiver, Sender},
    oneshot,
};

pub struct Dispatch<T>(Sender<Fun<T>>);

type Fun<T> = Box<dyn FnOnce(&mut T) + Send>;

impl<T> Clone for Dispatch<T> {
    fn clone(&self) -> Self {
        Dispatch(self.0.clone())
    }
}

impl<T> Dispatch<T> {
    pub async fn run<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&mut T) -> O + Send + 'static,
        O: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        let cb = |state: &mut T| {
            tx.send(f(state))
                .map_err(|_| ())
                .expect("oneshot send failed")
        };

        self.0
            .send(Box::new(cb))
            .await
            .expect("dispatch channel failure");

        rx.await.expect("should receive a value")
    }
}

pub fn dispatcher<T>(state: T) -> (Dispatch<T>, impl Future<Output = ()>) {
    let (tx, rx) = channel(1024);
    (Dispatch(tx), recv_loop(rx, state))
}

async fn recv_loop<T>(mut recv: Receiver<Fun<T>>, mut state: T) {
    while let Some(f) = recv.recv().await {
        f(&mut state);
    }
}
