use std::time::Duration;

use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::timeout;

const ADDR: &str = "127.0.0.1:6142";

#[tokio::main]
async fn main() -> io::Result<()> {
    let mut rng = rand::rng();
    let mut handles = JoinSet::new();
    for i in 0..10 {
        let sleep_secs = rand::Rng::random_range(&mut rng, 0..=10);
        handles.spawn(async move {
            tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
            run_client(i as usize, ADDR).await
        });
    }

    while let Some(res) = handles.join_next().await {
        if let Err(e) = res {
            eprintln!("Task failed: {e}");
        }
    }

    Ok(())
}

async fn run_client(index: usize, addr: &str) -> io::Result<()> {
    let socket = TcpStream::connect(addr).await?;
    let (mut rd, mut wr) = io::split(socket);

    let write_task = tokio::spawn(async move {
        println!("Client {index} sending message");
        wr.write_all(format!("hello world {index}").as_bytes())
            .await
    });

    let mut buf = vec![0; 128];

    loop {
        let result = timeout(Duration::from_secs(5), rd.read(&mut buf)).await;

        match result {
            Ok(Ok(0)) => {
                println!("Client {index} connection closed by server");
                break;
            }
            Ok(Ok(n)) => {
                let str = String::from_utf8_lossy(&buf[..n]);
                println!("GOT {str}");
            }
            Ok(Err(e)) => {
                eprintln!("Client {index} read error: {e}");
                break;
            }
            Err(_) => {
                eprintln!("Client {index} read timed out");
                break;
            }
        };
    }

    write_task.await??;

    Ok(())
}
