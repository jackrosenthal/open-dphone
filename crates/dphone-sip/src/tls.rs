//! SIP over TLS without C: `rustls` with the RustCrypto provider, plugged
//! into `rsipstack`'s platform TLS seam (its own `rustls` feature requires
//! `ring`, which needs a C toolchain for the target).
//!
//! Outgoing connections use the stack's TLS connector seam.  The stack has
//! no TLS listener without `ring` and no way to adopt an accepted TLS
//! stream, so [`listen`] accepts connections itself and joins each one to
//! the stack through its in-memory channel transport, using the stack's
//! own stream framing.

use crate::config::TlsConfig;
use rsipstack::platform::tls::{TlsClientConfig, TlsConnector, TlsStream};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream as RustlsStream;

static POLICY: OnceLock<Mutex<TlsConfig>> = OnceLock::new();

/// Installs (or updates) the process-wide TLS client policy and registers
/// the connector with the SIP stack.
pub fn install(cfg: TlsConfig) {
    let first = POLICY.get().is_none();
    let slot = POLICY.get_or_init(|| Mutex::new(TlsConfig::default()));
    *slot.lock().unwrap() = cfg;
    if first {
        rsipstack::platform::tls::set_client_connector(Arc::new(Connector));
    }
}

fn err(e: impl std::fmt::Display) -> rsipstack::Error {
    rsipstack::Error::Error(format!("tls: {e}"))
}

/// Accepts any server certificate but still checks the handshake
/// signature, like stock's default of not validating the PBX.
#[derive(Debug)]
struct NoVerify(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Builds the rustls client configuration for a policy.
pub fn client_config(policy: &TlsConfig) -> Result<ClientConfig, String> {
    let provider = Arc::new(rustls_rustcrypto::provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    if policy.verify {
        let mut roots = RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut &policy.roots_pem[..]).flatten() {
            let _ = roots.add(cert);
        }
        if roots.is_empty() {
            return Err("certificate checking is on but no trusted roots are configured".into());
        }
        Ok(builder.with_root_certificates(roots).with_no_client_auth())
    } else {
        Ok(builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth())
    }
}

/// The connector the SIP stack calls for `transport=tls` targets.
pub struct Connector;

#[async_trait::async_trait]
impl TlsConnector for Connector {
    async fn connect(&self, config: &TlsClientConfig, addr: SocketAddr) -> rsipstack::Result<Box<dyn TlsStream>> {
        let policy = POLICY.get().map(|p| p.lock().unwrap().clone()).unwrap_or_default();
        let stream = connect(&policy, &config.server_name, addr).await.map_err(err)?;
        Ok(Box::new(stream))
    }
}

/// Dials `addr` and completes a TLS handshake as `server_name`.
pub async fn connect(policy: &TlsConfig, server_name: &str, addr: SocketAddr) -> Result<Stream, String> {
    let cc = client_config(policy)?;
    let tcp = tokio::time::timeout(std::time::Duration::from_secs(10), TcpStream::connect(addr))
        .await
        .map_err(|_| format!("connecting to {addr} timed out"))?
        .map_err(|e| format!("cannot connect to {addr}: {e}"))?;
    let _ = tcp.set_nodelay(true);
    let local = tcp.local_addr().map_err(|e| e.to_string())?;
    let name = if server_name.is_empty() {
        ServerName::IpAddress(addr.ip().into())
    } else {
        ServerName::try_from(server_name.to_string()).unwrap_or(ServerName::IpAddress(addr.ip().into()))
    };
    let tls = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        tokio_rustls::TlsConnector::from(Arc::new(cc)).connect(name, tcp),
    )
    .await
    .map_err(|_| "the TLS handshake timed out".to_string())?
    .map_err(|e| format!("TLS handshake with {addr} failed: {e}"))?;
    let (r, w) = tokio::io::split(tls);
    Ok(Stream { read: tokio::sync::Mutex::new(r), write: tokio::sync::Mutex::new(w), local })
}

/// An established TLS connection.
pub struct Stream {
    read: tokio::sync::Mutex<ReadHalf<RustlsStream<TcpStream>>>,
    write: tokio::sync::Mutex<WriteHalf<RustlsStream<TcpStream>>>,
    local: SocketAddr,
}

#[async_trait::async_trait]
impl TlsStream for Stream {
    async fn recv(&self) -> rsipstack::Result<Vec<u8>> {
        let mut buf = vec![0u8; 4096];
        let n = self.read.lock().await.read(&mut buf).await.map_err(err)?;
        buf.truncate(n);
        Ok(buf)
    }

    async fn send_all(&self, data: Vec<u8>) -> rsipstack::Result<()> {
        let mut w = self.write.lock().await;
        w.write_all(&data).await.map_err(err)?;
        w.flush().await.map_err(err)
    }

    async fn shutdown(&self) -> rsipstack::Result<()> {
        self.write.lock().await.shutdown().await.map_err(err)
    }

    fn local_addr(&self) -> rsipstack::Result<SocketAddr> {
        Ok(self.local)
    }
}

/// Accepts SIP over TLS on `bind` and feeds the connections to the stack.
/// Returns once the listener is bound; the accept loop runs on.
pub async fn listen(
    bind: SocketAddr,
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
    inner: rsipstack::transaction::endpoint::EndpointInnerRef,
) -> Result<(), String> {
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(cert_der)], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)))
        .map_err(|e| format!("unusable TLS certificate or key: {e}"))?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind(bind).await.map_err(|e| e.to_string())?;
    log::info!("sip: accepting TLS on port {}", bind.port());
    tokio::spawn(async move {
        loop {
            let (tcp, remote) = match listener.accept().await {
                Ok(x) => x,
                Err(e) => {
                    log::warn!("sip: TLS accept: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    continue;
                }
            };
            let (acceptor, inner) = (acceptor.clone(), inner.clone());
            tokio::spawn(async move {
                let stream = match tokio::time::timeout(std::time::Duration::from_secs(20), acceptor.accept(tcp)).await {
                    Ok(Ok(s)) => s,
                    Ok(Err(e)) => return log::debug!("sip: TLS handshake from {remote} failed: {e}"),
                    Err(_) => return log::debug!("sip: TLS handshake from {remote} timed out"),
                };
                if let Err(e) = serve(stream, remote, inner).await {
                    log::debug!("sip: TLS connection from {remote} ended: {e}");
                }
            });
        }
    });
    Ok(())
}

/// Runs one accepted connection: frames what arrives with the stack's
/// stream codec and hands it in; writes out what the stack sends.
async fn serve(
    stream: tokio_rustls::server::TlsStream<TcpStream>,
    remote: SocketAddr,
    inner: rsipstack::transaction::endpoint::EndpointInnerRef,
) -> Result<(), String> {
    use rsipstack::transport::channel::ChannelConnection;
    use rsipstack::transport::stream::{SipCodec, SipCodecType};
    use rsipstack::transport::{SipAddr, SipConnection, TransportEvent};
    use tokio_util::codec::Decoder;

    let addr = SipAddr { r#type: Some(rsipstack::sip::Transport::Tls), addr: remote.into() };
    let (to_stack, from_us) = rsipstack::platform::mpsc::unbounded_channel();
    let (to_us, mut from_stack) = rsipstack::platform::mpsc::unbounded_channel();
    let channel = ChannelConnection::create_connection(from_us, to_us, addr.clone(), None).await.map_err(|e| e.to_string())?;
    let connection = SipConnection::Channel(channel);
    inner.transport_layer.add_connection(connection.clone());

    let (mut read, mut write) = tokio::io::split(stream);
    let mut buf = bytes::BytesMut::with_capacity(8192);
    let mut codec = SipCodec::new();
    let result = loop {
        tokio::select! {
            out = from_stack.recv() => match out {
                Some(TransportEvent::Incoming(msg, _, _)) => {
                    if let Err(e) = write.write_all(&msg.to_bytes()).await {
                        break Err(e.to_string());
                    }
                }
                Some(_) => {}
                None => break Ok(()),
            },
            n = read.read_buf(&mut buf) => match n {
                Ok(0) => break Ok(()),
                Ok(_) => loop {
                    match codec.decode(&mut buf) {
                        Ok(Some(SipCodecType::Message(msg))) => {
                            if to_stack.send(TransportEvent::Incoming(msg, connection.clone(), addr.clone())).is_err() {
                                return Ok(());
                            }
                        }
                        // RFC 5626 keepalive: answer the double CRLF.
                        Ok(Some(SipCodecType::KeepaliveRequest)) => {
                            let _ = write.write_all(b"\r\n").await;
                        }
                        Ok(Some(SipCodecType::KeepaliveResponse)) => {}
                        Ok(None) => break,
                        Err(e) => {
                            inner.transport_layer.remove_connection(&connection);
                            return Err(e.to_string());
                        }
                    }
                },
                Err(e) => break Err(e.to_string()),
            },
        }
    };
    inner.transport_layer.remove_connection(&connection);
    result
}
