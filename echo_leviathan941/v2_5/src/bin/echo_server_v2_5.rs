use futures::StreamExt;
use futures::stream::FuturesUnordered;

use tokio::io;
use tokio::net::TcpListener;

const MAX_CONNECTIONS_COUNT: usize = 3;

#[tokio::main]
async fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:6142").await?;
    let mut handlers = FuturesUnordered::new();

    loop {
        tokio::select! {
            Ok((mut stream, _addr)) = listener.accept() => {
                if handlers.len() < MAX_CONNECTIONS_COUNT {
                    let handler = tokio::spawn(async move {
                        let (mut rd, mut wr) = stream.split();
                        if io::copy(&mut rd, &mut wr).await.is_err() {
                            eprintln!("Failed to copy");
                        }
                    });
                    handlers.push(handler);
                } else {
                    println!("Max connections reached, rejecting connection");
                }
            }

            Some(_) = handlers.next() => {
                println!("A connection handler finished");
            }

            else => {
                // No more events to process
                break;
            }
        }
    }
    Ok(())
}
