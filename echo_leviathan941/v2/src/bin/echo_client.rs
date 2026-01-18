use std::time::Duration;

use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::timeout;

const ADDR: &str = "127.0.0.1:6142";

#[tokio::main]
async fn main() -> io::Result<()> {
    let mut handles = JoinSet::new();
    for i in 0..5 {
        handles.spawn(async move {
            run_client(i, ADDR).await
        });
    }

    while let Some(res) = handles.join_next().await {
        if let Err(e) = res {
            eprintln!("Task failed: {}", e);
        }
    }

    Ok(())
}

async fn run_client(index: usize, addr: &str) -> io::Result<()> {
    let socket = TcpStream::connect(addr).await?;
    let (mut rd, mut wr) = io::split(socket);

    let write_task = tokio::spawn(async move {
        println!("Client {} sending message", index);
        wr.write_all(format!("hello world {}", index).as_bytes()).await
    });

    let mut buf = vec![0; 128];

    loop {
        let result = timeout(Duration::from_secs(5), rd.read(&mut buf)).await;

        match result {
            Ok(Ok(n)) => {
                if let Ok(str) = std::str::from_utf8(&buf[..n]) {
                    println!("GOT a string: {}", str);
                } else {
                    println!("GOT {:?}", &buf[..n]);
                }
            },
            Ok(Err(e)) => {
                eprintln!("Client {} read error: {}", index, e);
                break;
            }
            Err(_) => {
                eprintln!("Client {} read timed out", index);
                break;
            }
        };
    }

    write_task.await??;

    Ok(())
}
