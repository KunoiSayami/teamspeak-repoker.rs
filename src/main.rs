use crate::datastructures::poked::NotifyClientPoke;
use crate::datastructures::SocketConn;
use anyhow::anyhow;
use clap::{arg, Command};
use log::{debug, error, info};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot::Receiver;
use tokio::sync::Mutex;

mod datastructures;

async fn real_staff(
    mut conn: SocketConn,
    mut recv: Receiver<bool>,
    notify_signal: Arc<Mutex<bool>>,
) -> anyhow::Result<()> {
    loop {
        if recv.try_recv().is_ok() {
            info!("Exit!");
            return Ok(());
        }
        let data = conn.read_data().await?;
        if data.is_none() {
            let mut signal = notify_signal.lock().await;
            if *signal {
                conn.who_am_i().await?;
                *signal = false;
            }
            continue;
        }
        let data = data.unwrap();
        for line in data.lines() {
            if !line.contains("notifyclientpoke") {
                continue;
            }
            let poker: NotifyClientPoke = serde_teamspeak_querystring::from_str(
                line.split_once("notifyclientpoke ")
                    .ok_or_else(|| anyhow!("Can't found prefix: {:?}", line))?
                    .1,
            )
            .map_err(|_e| anyhow!("Got error while parse data: {:?}", line))?;
            conn.select_server(poker.server_handler_id()).await??;

            conn.poke(poker.invoker_id()).await??;
            info!("Re poke: {}", poker.invoker_name());
        }
    }
}

async fn staff(api_key: &str, server: &str, port: u16) -> anyhow::Result<()> {
    let mut conn = SocketConn::connect(server, port)
        .await
        .map_err(|e| anyhow!("Connect error: {:?}", e))?;
    conn.login(api_key).await??;
    conn.register_events().await??;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    let keepalive_signal = Arc::new(Mutex::new(false));
    let alt_signal = keepalive_signal.clone();
    tokio::select! {
        _ = async move {
            tokio::signal::ctrl_c().await.unwrap();
            sender.send(true).unwrap();
            info!("Recv SIGINT signal, send exit signal");
            tokio::signal::ctrl_c().await.unwrap();
            info!("Recv SIGINT again, force exit.");
            std::process::exit(137);
        } => {}
        _ = async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let mut i = keepalive_signal.lock().await;
                *i = true;
            }
        } => {}
        ret = real_staff(conn, receiver, alt_signal) => {
           ret?;
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let matches = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .args(&[
            arg!([API_KEY] "Teamspeak client query api key"),
            arg!(--server "Specify server"),
            arg!(--port "Specify port"),
        ])
        .get_matches();

    env_logger::Builder::from_default_env().init();

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(staff(
            matches.value_of("API_KEY").unwrap(),
            matches.value_of("server").unwrap_or("localhost"),
            matches
                .value_of("port")
                .unwrap_or("25639")
                .parse()
                .unwrap_or_else(|e| {
                    error!("Got parse error, use default 25639 instead. {:?}", e);
                    25639
                }),
        ))?;

    Ok(())
}
