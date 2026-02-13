//! Saved host configuration for remote terminal connections.

use serde::Deserialize;

/// A saved remote host entry.
#[derive(Debug, Clone, Deserialize)]
pub struct HostEntry {
    /// Human-readable name (e.g., "briefcase", "dev-server").
    pub name: String,
    /// IP address or hostname.
    pub address: String,
    /// TCP port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Connection protocol hint.
    #[serde(default = "default_protocol")]
    pub protocol: String,
    /// Optional PSK for authentication.
    #[serde(default)]
    pub psk: Option<String>,
}

fn default_port() -> u16 {
    9000
}

fn default_protocol() -> String {
    "oasis-terminal".to_string()
}

/// Parse a hosts TOML file into a list of host entries.
pub fn parse_hosts(toml_str: &str) -> oasis_types::error::Result<Vec<HostEntry>> {
    #[derive(Deserialize)]
    struct HostsFile {
        #[serde(default)]
        host: Vec<HostEntry>,
    }

    let file: HostsFile = toml::from_str(toml_str)
        .map_err(|e| oasis_types::error::OasisError::Config(format!("hosts.toml: {e}")))?;
    Ok(file.host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_port() {
        assert_eq!(default_port(), 9000);
    }

    #[test]
    fn test_default_protocol() {
        assert_eq!(default_protocol(), "oasis-terminal");
    }

    #[test]
    fn test_parse_hosts_empty() {
        let toml = "";
        let result = parse_hosts(toml);
        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 0);
    }

    #[test]
    fn test_parse_hosts_single_entry() {
        let toml = r#"
[[host]]
name = "dev-server"
address = "192.168.1.100"
"#;
        let result = parse_hosts(toml);
        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "dev-server");
        assert_eq!(hosts[0].address, "192.168.1.100");
        assert_eq!(hosts[0].port, 9000); // default
        assert_eq!(hosts[0].protocol, "oasis-terminal"); // default
        assert!(hosts[0].psk.is_none());
    }

    #[test]
    fn test_parse_hosts_with_custom_port() {
        let toml = r#"
[[host]]
name = "briefcase"
address = "10.0.0.5"
port = 8080
"#;
        let result = parse_hosts(toml);
        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "briefcase");
        assert_eq!(hosts[0].port, 8080);
    }

    #[test]
    fn test_parse_hosts_with_psk() {
        let toml = r#"
[[host]]
name = "secure-server"
address = "example.com"
psk = "secret123"
"#;
        let result = parse_hosts(toml);
        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "secure-server");
        assert_eq!(hosts[0].address, "example.com");
        assert_eq!(hosts[0].psk, Some("secret123".to_string()));
    }

    #[test]
    fn test_parse_hosts_with_protocol() {
        let toml = r#"
[[host]]
name = "ftp-server"
address = "ftp.example.com"
protocol = "ftp"
"#;
        let result = parse_hosts(toml);
        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].protocol, "ftp");
    }

    #[test]
    fn test_parse_hosts_multiple_entries() {
        let toml = r#"
[[host]]
name = "server1"
address = "192.168.1.1"

[[host]]
name = "server2"
address = "192.168.1.2"
port = 7777

[[host]]
name = "server3"
address = "example.org"
psk = "key123"
"#;
        let result = parse_hosts(toml);
        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 3);
        assert_eq!(hosts[0].name, "server1");
        assert_eq!(hosts[1].name, "server2");
        assert_eq!(hosts[1].port, 7777);
        assert_eq!(hosts[2].name, "server3");
        assert_eq!(hosts[2].psk, Some("key123".to_string()));
    }

    #[test]
    fn test_parse_hosts_all_fields() {
        let toml = r#"
[[host]]
name = "full-config"
address = "192.168.100.50"
port = 9999
protocol = "custom"
psk = "my-secret"
"#;
        let result = parse_hosts(toml);
        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "full-config");
        assert_eq!(hosts[0].address, "192.168.100.50");
        assert_eq!(hosts[0].port, 9999);
        assert_eq!(hosts[0].protocol, "custom");
        assert_eq!(hosts[0].psk, Some("my-secret".to_string()));
    }

    #[test]
    fn test_parse_hosts_invalid_toml() {
        let toml = "this is not valid toml [[[";
        let result = parse_hosts(toml);
        assert!(result.is_err());
        if let Err(oasis_types::error::OasisError::Config(msg)) = result {
            assert!(msg.contains("hosts.toml"));
        } else {
            panic!("Expected Config error");
        }
    }

    #[test]
    fn test_parse_hosts_missing_required_fields() {
        let toml = r#"
[[host]]
name = "incomplete"
"#;
        // Missing 'address' field
        let result = parse_hosts(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_hosts_hostname_instead_of_ip() {
        let toml = r#"
[[host]]
name = "dns-host"
address = "my-server.local"
"#;
        let result = parse_hosts(toml);
        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert_eq!(hosts[0].address, "my-server.local");
    }

    #[test]
    fn test_parse_hosts_empty_psk() {
        let toml = r#"
[[host]]
name = "no-auth"
address = "192.168.1.1"
psk = ""
"#;
        let result = parse_hosts(toml);
        assert!(result.is_ok());
        let hosts = result.unwrap();
        // Empty string is still Some, not None
        assert_eq!(hosts[0].psk, Some("".to_string()));
    }

    #[test]
    fn test_host_entry_clone() {
        let entry = HostEntry {
            name: "test".to_string(),
            address: "127.0.0.1".to_string(),
            port: 8080,
            protocol: "custom".to_string(),
            psk: Some("key".to_string()),
        };
        let cloned = entry.clone();
        assert_eq!(entry.name, cloned.name);
        assert_eq!(entry.address, cloned.address);
        assert_eq!(entry.port, cloned.port);
        assert_eq!(entry.protocol, cloned.protocol);
        assert_eq!(entry.psk, cloned.psk);
    }

    #[test]
    fn test_host_entry_debug() {
        let entry = HostEntry {
            name: "debug-test".to_string(),
            address: "10.0.0.1".to_string(),
            port: 9000,
            protocol: "oasis-terminal".to_string(),
            psk: None,
        };
        let debug_str = format!("{:?}", entry);
        assert!(debug_str.contains("debug-test"));
        assert!(debug_str.contains("10.0.0.1"));
    }
}
