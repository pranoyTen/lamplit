// Whether the internet connection is metered (Windows: marked as metered in Settings, roaming or over its data limit), so downloads
// can wait for an unmetered one unless Settings allows them.

#[cfg(windows)]
pub fn is_metered() -> bool {
    use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
    let cost = || -> windows::core::Result<bool> {
        let c = NetworkInformation::GetInternetConnectionProfile()?.GetConnectionCost()?;
        let t = c.NetworkCostType()?;
        Ok(t == NetworkCostType::Fixed || t == NetworkCostType::Variable || c.Roaming()? || c.OverDataLimit()?)
    };
    cost().unwrap_or(false) // no connection profile: nothing to protect
}

#[cfg(not(windows))]
pub fn is_metered() -> bool {
    false // shortcut: macOS reports this through NWPathMonitor (isExpensive / isConstrained); add it with the Mac build
}
