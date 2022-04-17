use crate::datastructures::poked::NotifyClientPoke;
use crate::datastructures::SocketConn;
use anyhow::anyhow;
use clap::{arg, Command};
use log::{error, info};

mod datastructures;

async fn staff(api_key: &str, server: &str, port: u16) -> anyhow::Result<()> {
    let mut conn = SocketConn::connect(server, port)
        .await
        .map_err(|e| anyhow!("Connect error: {:?}", e))?;
    let status = conn.login(api_key).await?;
    if status.is_err() {
        return Err(anyhow!("Got error in login: {:?}", status));
    }
    let status = conn.register_events().await?;
    if status.is_err() {
        return Err(anyhow!("Got error while register events: {:?}", status));
    }

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Recv SIGINT signal, exit.");
                return Ok(())
            }
            // TODO: Should keep connection alive.
            data = conn.read_data() => {
                let data = data?;
                if data.is_none() {
                    continue;
                }
                let data = data.unwrap();
                if !data.contains("notifyclientpoke") {
                    continue;
                }
                let poker: NotifyClientPoke = serde_teamspeak_querystring::from_str(
                    data.split_once("notifyclientpoke ")
                        .ok_or_else(|| anyhow!("Can't found prefix: {:?}", data))?
                        .1,
                )
                .map_err(|e| anyhow!("Got error while parse data: {:?}", data))?;

                let status = conn.select_server(poker.server_handler_id()).await?;
                if status.is_err() {
                    return Err(anyhow!("Got error while select server: {:?}", status));
                }

                let status = conn.poke(poker.invoker_id()).await?;
                if status.is_err() {
                    return Err(anyhow!("Got error while poke client: {:?}", status));
                }
                info!("Re poke: {}", poker.invoker_name());
            }
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
