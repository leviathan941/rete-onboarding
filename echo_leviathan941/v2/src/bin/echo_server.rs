use std::sync::Arc;

use tokio::io;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

const MAX_CONNECTIONS_COUNT: usize = 3;

#[tokio::main]
async fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:6142").await?;
    let semaphore = Arc::new(Semaphore::new(MAX_CONNECTIONS_COUNT));

    loop {
        let (mut socket, _) = listener.accept().await?;
        if let Ok(permit) = semaphore.clone().try_acquire_owned() {
            tokio::spawn(async move {
                let _permit = permit;
                let (mut rd, mut wr) = socket.split();

                if io::copy(&mut rd, &mut wr).await.is_err() {
                    eprintln!("failed to copy");
                }
            });
        } else {
            println!("Max connections {} reached; dropping new connection.", MAX_CONNECTIONS_COUNT);
            continue;
        }
    }
}
