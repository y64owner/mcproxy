#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{copy_bidirectional, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::interval;

const LISTEN_ADDR: &str = "0.0.0.0:25565";
const ORIGIN_ADDR: &str = "127.0.0.1:25565";
const MAX_CONNS_PER_IP: usize = 20;
const RATE_LIMIT_MAX: usize = 40;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(10);
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Default)]
struct IpEntry {
    active: usize,
    recent: Vec<Instant>,
}

type Table = Arc<Mutex<HashMap<IpAddr, IpEntry>>>;

#[tokio::main]
async fn main() {
    let origin = tokio::net::lookup_host(ORIGIN_ADDR)
        .await
        .expect("cannot resolve ORIGIN_ADDR")
        .next()
        .expect("ORIGIN_ADDR resolved to nothing");

    let listener = TcpListener::bind(LISTEN_ADDR)
        .await
        .expect("cannot bind LISTEN_ADDR");

    println!("mcproxy on {LISTEN_ADDR} -> {origin}");

    let table: Table = Arc::new(Mutex::new(HashMap::new()));
    spawn_sweeper(table.clone());

    loop {
        match listener.accept().await {
            Ok((inbound, peer)) => {
                let table = table.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle(inbound, peer, origin, table).await {
                        eprintln!("{} {e}", peer.ip());
                    }
                });
            }
            Err(e) => {
                eprintln!("accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn handle(
    mut inbound: TcpStream,
    peer: SocketAddr,
    origin: SocketAddr,
    table: Table,
) -> io::Result<()> {
    if !admit(&table, peer.ip()) {
        return Err(io::Error::other("rejected"));
    }

    let result = pipe(&mut inbound, peer, origin).await;
    release(&table, peer.ip());
    result
}

fn admit(table: &Table, ip: IpAddr) -> bool {
    let now = Instant::now();
    let mut map = table.lock().unwrap();
    let entry = map.entry(ip).or_default();
    entry
        .recent
        .retain(|t| now.duration_since(*t) < RATE_LIMIT_WINDOW);
    if entry.recent.len() >= RATE_LIMIT_MAX || entry.active >= MAX_CONNS_PER_IP {
        return false;
    }
    entry.recent.push(now);
    entry.active += 1;
    true
}

fn release(table: &Table, ip: IpAddr) {
    let mut map = table.lock().unwrap();
    if let Some(entry) = map.get_mut(&ip) {
        entry.active = entry.active.saturating_sub(1);
    }
}

async fn pipe(inbound: &mut TcpStream, peer: SocketAddr, origin: SocketAddr) -> io::Result<()> {
    inbound.set_nodelay(true)?;

    let mut probe = [0u8; 1];
    match tokio::time::timeout(FIRST_BYTE_TIMEOUT, inbound.peek(&mut probe)).await {
        Ok(Ok(n)) if n > 0 => {}
        Ok(Ok(_)) => return Err(io::Error::other("closed before first byte")),
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(io::Error::other("idle before first byte")),
    }

    let connect = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(origin));
    let mut outbound = match connect.await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(io::Error::other("origin connect timeout")),
    };
    outbound.set_nodelay(true)?;
    outbound
        .write_all(&proxy_v2_header(peer, inbound.local_addr()?))
        .await?;

    let (up, down) = copy_bidirectional(inbound, &mut outbound).await?;
    println!("{} up={up} down={down}", peer.ip());
    Ok(())
}

fn proxy_v2_header(client: SocketAddr, proxy: SocketAddr) -> Vec<u8> {
    const SIG: [u8; 12] = [
        0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
    ];

    let mut buf = Vec::with_capacity(28);
    buf.extend_from_slice(&SIG);

    match (client.ip(), proxy.ip()) {
        (IpAddr::V4(src), IpAddr::V4(dst)) => {
            buf.push(0x21);
            buf.push(0x11);
            buf.extend_from_slice(&12u16.to_be_bytes());
            buf.extend_from_slice(&src.octets());
            buf.extend_from_slice(&dst.octets());
            buf.extend_from_slice(&client.port().to_be_bytes());
            buf.extend_from_slice(&proxy.port().to_be_bytes());
        }
        _ => {
            buf.push(0x20);
            buf.push(0x00);
            buf.extend_from_slice(&0u16.to_be_bytes());
        }
    }
    buf
}

fn spawn_sweeper(table: Table) {
    tokio::spawn(async move {
        let mut tick = interval(SWEEP_INTERVAL);
        loop {
            tick.tick().await;
            let now = Instant::now();
            let mut map = table.lock().unwrap();
            map.retain(|_, entry| {
                entry
                    .recent
                    .retain(|t| now.duration_since(*t) < RATE_LIMIT_WINDOW);
                entry.active > 0 || !entry.recent.is_empty()
            });
        }
    });
}
