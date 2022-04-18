pub mod socket {
    use super::{FromQueryString, QueryResult, QueryStatus};
    use anyhow::anyhow;
    use log::{error, warn};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    const BUFFER_SIZE: usize = 512;

    pub struct SocketConn {
        conn: TcpStream,
    }

    impl SocketConn {
        fn decode_status(content: String) -> anyhow::Result<(Option<QueryStatus>, String)> {
            /*debug_assert!(
                !content.contains("Welcome to the TeamSpeak 3") && content.contains("error id="),
                "Content => {:?}",
                content
            );*/

            for line in content.lines() {
                if line.trim().starts_with("error ") {
                    let status = QueryStatus::try_from(line)?;

                    return Ok((Some(status), content));
                }
            }
            Ok((None, content))
        }

        pub async fn read_data(&mut self) -> anyhow::Result<Option<String>> {
            let mut buffer = [0u8; BUFFER_SIZE];
            let mut ret = String::new();
            loop {
                let size = if let Ok(data) =
                    tokio::time::timeout(Duration::from_secs(2), self.conn.read(&mut buffer)).await
                {
                    match data {
                        Ok(size) => size,
                        Err(e) => return Err(anyhow!("Got error while read data: {:?}", e)),
                    }
                } else {
                    return Ok(None);
                };

                ret.push_str(&String::from_utf8_lossy(&buffer[..size]));
                if size < BUFFER_SIZE || (ret.contains("error id=") && ret.ends_with("\n\r")) {
                    break;
                }
            }
            Ok(Some(ret))
        }

        async fn write_data(&mut self, payload: &str) -> anyhow::Result<()> {
            debug_assert!(payload.ends_with("\n\r"));
            self.conn
                .write(payload.as_bytes())
                .await
                .map(|size| {
                    if size != payload.as_bytes().len() {
                        error!(
                            "Error payload size mismatch! expect {} but {} found. payload: {:?}",
                            payload.as_bytes().len(),
                            size,
                            payload
                        )
                    }
                })
                .map_err(|e| anyhow!("Got error while send data: {:?}", e))?;
            /*self.conn
            .flush()
            .await
            .map_err(|e| anyhow!("Got error while flush data: {:?}", e))?;*/
            Ok(())
        }

        async fn basic_operation(&mut self, payload: &str) -> anyhow::Result<QueryResult<()>> {
            let data = self.write_and_read(payload).await?;
            Self::decode_status(dbg!(data))?
                .0
                .ok_or_else(|| anyhow!("Can't find status line."))
                .map(|status| {
                    status.into_result(
                        payload
                            .trim()
                            .split_once(' ')
                            .unwrap_or((payload.trim(), ""))
                            .0,
                        (),
                    )
                })
        }

        async fn write_and_read(&mut self, payload: &str) -> anyhow::Result<String> {
            self.write_data(payload).await?;
            let mut buffer = String::new();
            loop {
                let ret = self.read_data().await?;
                if ret.is_none() {
                    break;
                }
                let ret = ret.unwrap();

                buffer.push_str(&ret);
                if ret.contains("error ") {
                    break;
                }
            }
            if buffer.is_empty() {
                return Err(anyhow!("Return data is none"));
            }
            Ok(buffer)
        }

        pub async fn connect(server: &str, port: u16) -> anyhow::Result<Self> {
            let conn = TcpStream::connect(format!("{}:{}", server, port))
                .await
                .map_err(|e| anyhow!("Got error while connect to {}:{} {:?}", server, port, e))?;

            let mut self_ = Self { conn };

            tokio::time::sleep(Duration::from_millis(10)).await;

            let content = self_
                .read_data()
                .await
                .map_err(|e| anyhow!("Got error in connect while read content: {:?}", e))?;

            if content.is_none() {
                warn!("Read none data.");
            }

            Ok(self_)
        }

        pub async fn poke(&mut self, clid: i64) -> anyhow::Result<QueryResult<()>> {
            self.basic_operation(&format!("clientpoke clid={} msg=\n\r", clid))
                .await
        }

        pub async fn register_events(&mut self) -> anyhow::Result<QueryResult<()>> {
            self.basic_operation("clientnotifyregister schandlerid=0 event=notifyclientpoke\n\r")
                .await
        }

        pub async fn login(&mut self, api_key: &str) -> anyhow::Result<QueryResult<()>> {
            let payload = format!("auth apikey={}\n\r", api_key);
            self.basic_operation(payload.as_str()).await
        }

        pub async fn select_server(&mut self, server_id: i64) -> anyhow::Result<QueryResult<()>> {
            let payload = format!("use {}\n\r", server_id);
            self.basic_operation(payload.as_str()).await
        }

        pub async fn who_am_i(&mut self) -> anyhow::Result<()> {
            self.write_data("whoami\n\r").await
        }
    }
}
pub trait FromQueryString: for<'de> Deserialize<'de> {
    fn from_query(data: &str) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        serde_teamspeak_querystring::from_str(data)
            .map_err(|e| anyhow::anyhow!("Got parser error: {:?}", e))
    }
}
pub mod poked {
    use serde_derive::Deserialize;

    #[allow(dead_code)]
    #[derive(Clone, Debug, Deserialize)]
    pub struct NotifyClientPoke {
        #[serde(rename = "schandlerid")]
        server_handler_id: i64,
        #[serde(rename = "invokerid")]
        invoker_id: i64,
        #[serde(rename = "invokername")]
        invoker_name: String,
        #[serde(default)]
        msg: String,
    }

    impl NotifyClientPoke {
        pub fn server_handler_id(&self) -> i64 {
            self.server_handler_id
        }
        pub fn invoker_id(&self) -> i64 {
            self.invoker_id
        }
        pub fn invoker_name(&self) -> &str {
            &self.invoker_name
        }
    }
}

pub mod select_result {
    use serde_derive::Deserialize;

    #[allow(dead_code)]
    #[derive(Clone, Debug, Deserialize)]
    pub struct SelectResult {
        id: i32,
        msg: String,
    }
}

pub mod query_status {
    use crate::datastructures::{QueryError, QueryResult};
    use anyhow::anyhow;
    use serde_derive::Deserialize;

    #[allow(dead_code)]
    #[derive(Clone, Debug, Deserialize)]
    pub struct QueryStatus {
        id: i32,
        msg: String,
    }

    impl Default for QueryStatus {
        fn default() -> Self {
            Self {
                id: 0,
                msg: "ok".to_string(),
            }
        }
    }

    impl QueryStatus {
        pub fn is_ok(&self) -> bool {
            self.id == 0
        }

        pub fn into_error(self, method: &str) -> QueryError {
            QueryError::new(method, self.id, self.msg)
        }

        pub fn into_result<T: Send>(self, method: &str, data: T) -> QueryResult<T> {
            if self.is_ok() {
                Ok(data)
            } else {
                Err(self.into_error(method))
            }
        }
    }

    impl TryFrom<&str> for QueryStatus {
        type Error = anyhow::Error;

        fn try_from(value: &str) -> Result<Self, Self::Error> {
            let (_, line) = value
                .split_once("error ")
                .ok_or_else(|| anyhow!("Split error: {}", value))?;
            serde_teamspeak_querystring::from_str(line)
                .map_err(|e| anyhow!("Got error while parse string: {:?} {:?}", line, e))
        }
    }
}

pub mod query_result {
    use serde_derive::Deserialize;
    use std::fmt::Formatter;

    pub type Result<T> = std::result::Result<T, QueryError>;

    #[derive(Debug, Deserialize)]
    pub struct QueryError {
        method: String,
        id: i32,
        msg: String,
    }

    impl QueryError {
        pub fn new(method: &str, id: i32, msg: String) -> Self {
            Self {
                method: method.to_string(),
                id,
                msg,
            }
        }
    }

    impl std::error::Error for QueryError {}

    impl std::fmt::Display for QueryError {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "Call method: {} error! Code: {}({})",
                self.method, self.id, self.msg
            )
        }
    }
}

pub use query_result::QueryError;
pub use query_result::Result as QueryResult;
pub use query_status::QueryStatus;
pub use select_result::SelectResult;
use serde::Deserialize;
pub use socket::SocketConn;
