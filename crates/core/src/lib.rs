//! Shared building blocks of USB-IP-Bridge: version, device model, configuration,
//! the local API between service and GUI/CLI, and logging.

pub mod api;
pub mod classes;
pub mod config;
pub mod logging;
pub mod model;
pub mod peer;
pub mod profiles;
pub mod secure;

/// Application version, e.g. "0.0.1" (single source: workspace Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Product name as shown to users.
pub const PRODUCT_NAME: &str = "USB-IP-Bridge";

/// Window title / banner, e.g. "USB-IP-Bridge v0.0.1".
pub fn title() -> String {
    format!("{PRODUCT_NAME} v{VERSION}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn title_contains_version() {
        assert_eq!(super::title(), format!("USB-IP-Bridge v{}", env!("CARGO_PKG_VERSION")));
    }
}
