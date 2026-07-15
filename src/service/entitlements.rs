use crate::service::capabilities::AccountCapabilities;

pub enum Feature {
    Tsa,
    ServerBackup,
    Identity,
    HistoryRecovery,
}

pub fn allowed(capabilities: &AccountCapabilities, feature: Feature) -> bool {
    match feature {
        Feature::Tsa => capabilities.tsa_available(),
        Feature::ServerBackup => capabilities.server_backup,
        Feature::Identity => capabilities.identity_enabled,
        Feature::HistoryRecovery => capabilities.history_recovery,
    }
}
