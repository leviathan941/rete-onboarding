use std::time::Duration;

use futures::stream::StreamExt;

use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::{interval, timeout};
use tokio_stream::wrappers::IntervalStream;

const ADDR: &str = "127.0.0.1:6142";

#[tokio::main]
async fn main() -> io::Result<()> {
    let mut handles = JoinSet::new();
    let task_interval = interval(Duration::from_secs(1));
    let mut stream = IntervalStream::new(task_interval).enumerate().take(10);
    while let Some((i, _)) = stream.next().await {
        handles.spawn(async move { run_connection(i, ADDR).await });
    }

    while let Some(res) = handles.join_next().await {
        if let Err(e) = res {
            eprintln!("Task failed: {e}");
        }
    }

    Ok(())
}

async fn run_connection(index: usize, addr: &str) -> io::Result<()> {
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
