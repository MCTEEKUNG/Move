/// mDNS auto-discovery of NetShare servers on the LAN.
///
/// Service type: `_netshare._tcp.local.`
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

pub const SERVICE_TYPE: &str = "_netshare._tcp.local.";

#[derive(Clone, Debug)]
pub struct FoundServer {
    pub name: String,
    pub addr: String,
    pub port: u16,
}

/// Browse for servers on the LAN; call `poll()` each frame to collect results.
pub struct MdnsBrowser {
    _daemon: ServiceDaemon,
    receiver: mdns_sd::Receiver<ServiceEvent>,
    pub servers: Vec<FoundServer>,
}

impl MdnsBrowser {
    pub fn start() -> anyhow::Result<Self> {
        let daemon   = ServiceDaemon::new()?;
        let receiver = daemon.browse(SERVICE_TYPE)?;
        Ok(Self { _daemon: daemon, receiver, servers: Vec::new() })
    }

    /// Drain pending mDNS events; must be called from the GUI frame loop.
    pub fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    if let Some(addr) = info.get_addresses().iter().next() {
                        let entry = FoundServer {
                            name: info.get_fullname().to_owned(),
                            addr: addr.to_string(),
                            port: info.get_port(),
                        };
                        // Deduplicate.
                        if !self.servers.iter().any(|s| s.name == entry.name) {
                            self.servers.push(entry);
                        }
                    }
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    self.servers.retain(|s| s.name != fullname);
                }
                _ => {}
            }
        }
    }
}

/// Advertise this machine as a NetShare server (call from server mode).
pub struct MdnsAdvertiser {
    daemon: ServiceDaemon,
}

impl MdnsAdvertiser {
    pub fn start(hostname: &str, port: u16) -> anyhow::Result<Self> {
        let daemon  = ServiceDaemon::new()?;
        // Try to get the local IP; fall back to 0.0.0.0 (mDNS will bind properly).
        let local_ip = local_ip().unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));

        let instance = format!("NetShare-{hostname}");
        let host_fqdn = format!("{hostname}.local.");
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &host_fqdn,
            local_ip,
            port,
            None,
        )?;
        daemon.register(service)?;
        Ok(Self { daemon })
    }
}

impl Drop for MdnsAdvertiser {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

impl MdnsAdvertiser {
    /// A no-op advertiser for when mDNS fails to initialise.
    pub fn dummy() -> Self {
        Self { daemon: ServiceDaemon::new().expect("mdns daemon") }
    }
}

fn local_ip() -> Option<std::net::IpAddr> {
    // Connect a UDP socket to get the local outbound IP without actually sending.
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    Some(sock.local_addr().ok()?.ip())
}
