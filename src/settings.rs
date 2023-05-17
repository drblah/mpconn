use serde::Deserialize;
use std::net::{Ipv4Addr, SocketAddr};

/// LocalTypes defines the variants of Local which can be specified in the settings file.
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum LocalTypes {
    /// The Layer2 Local requires 1 parameter in the form of a network interface name. For example
    /// 'eth0', 'wlan0' etc.

    Layer2 {
        network_interface: String,
    },

    /// The Layer3 Local variant requires 1 parameter in the form of an IP address which will be assigned
    /// to the TUN device, mpconn creates if Layer 3 tunneling is used. This IP address must be
    /// globally unique amongst all connected Peers.
    Layer3 {
        tun_ip: Ipv4Addr
    },
}

/// RemoteTypes specifies the variant of Remotes which can be specified in the settings file.
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum RemoteTypes {
    /// The UDP Remote variant takes a mandatory interface name onto which a UDP socket will be bound.

    UDP {
        iface: String,
        /// Optionally, the user can provide an IP address on which the socket will bind.
        listen_addr: Option<Ipv4Addr>,
        /// The listen port is mandatory and specifies which port the UDP socket will bind to.
        listen_port: u16,
        /// Whether the Remote should bind to the device or not. This is mostly useful for a server
        /// instance which only has a single interface with multiple routes to clients. Default is true
        bind_to_device: Option<bool>,

        /// Device metric
        metrics: MetricConfig,
    },

    /// The UDPLz4 Remote variant is identical to the UDP Remote in most ways.
    UDPLz4 {
        iface: String,
        /// Optionally, the user can provide an IP address on which the socket will bind.
        listen_addr: Option<Ipv4Addr>,
        /// The listen port is mandatory and specifies which port the UDP socket will bind to.
        listen_port: u16,
        /// Whether the Remote should bind to the device or not. This is mostly useful for a server
        /// instance which only has a single interface with multiple routes to clients. Default is true
        bind_to_device: Option<bool>,

        /// Device metric
        metrics: MetricConfig,
    },
}

/// PeerConfig contains information about Peers to which, mpconn should connect
#[derive(Deserialize, Debug, Clone)]
pub struct PeerConfig {
    pub addresses: Vec<SocketAddr>,
    /// must contain the unique id of the peer.
    pub id: u16,
}

/// Network metrics obtained from network interfaces
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum MetricConfig {
    /// This metric does nothing. When its Watch channel is pulled it will block indefinitely.
    Nothing {},
    Nr5gSignal {},
}

/// A settings file can contain zero or more peers which will be used to seed the PeerList. An
/// mpconn server will typically not have any peers defined in the settings file. A client must have
/// one or more peers defined in the settings file. Otherwise it will not know who to connect to.
#[derive(Deserialize, Debug, Clone)]
pub struct SettingsFile {
    /// One or more Peer configurations used to seed the PeerList
    pub peers: Vec<PeerConfig>,
    /// The keep alive interval at which the mpconn instance should send keep alive messages to its
    /// peers
    pub keep_alive_interval: u64,
    /// The type of Local mpconn should configure. Only one local can be configured at any given
    /// time
    pub local: LocalTypes,
    /// One or more Remote which mpconn should be using for performing multi-connectivity.
    pub remotes: Vec<RemoteTypes>,
    /// The unique peer ID of this mpconn instance. This ID *MUST* be unique in amongst all
    /// connected peers.
    pub peer_id: u16,
}
