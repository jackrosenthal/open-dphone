//! DPMA over `dphone-sip`: out-of-dialog MESSAGE in both directions.

use crate::{Message, Transport};
use dphone_sip::{OutMessage, SipHandle};
use std::sync::Arc;
use tokio::sync::mpsc;

struct SipTransport {
    handle: SipHandle,
    mac: String,
}

#[async_trait::async_trait]
impl Transport for SipTransport {
    async fn send(&self, server_uri: &str, message: &Message) -> Result<(), String> {
        let code = self
            .handle
            .send_message(OutMessage {
                account: None,
                to: server_uri.to_string(),
                // Like stock, the session's Contact and From are the MAC
                // at the phone's own address, the same for every message.
                from_user: Some(self.mac.clone()),
                headers: message
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), String::from_utf8_lossy(v).into_owned()))
                    .collect(),
                content_type: "text/plain".into(),
                body: message.body.clone(),
            })
            .await?;
        if (200..300).contains(&code) { Ok(()) } else { Err(format!("the MESSAGE was refused: {code}")) }
    }
}

/// The transport and the incoming stream to pass to [`crate::start`].
pub fn transport(handle: &SipHandle, mac: &str) -> (Arc<dyn Transport>, mpsc::UnboundedReceiver<Message>) {
    let mut inbox = handle.messages();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(m) = inbox.recv().await {
            let message = Message {
                headers: m.headers.into_iter().map(|(k, v)| (k, v.into_bytes())).collect(),
                body: m.body,
            };
            if tx.send(message).is_err() {
                break;
            }
        }
    });
    let mac = mac.to_ascii_lowercase().replace([':', '-'], "");
    (Arc::new(SipTransport { handle: handle.clone(), mac }), rx)
}
