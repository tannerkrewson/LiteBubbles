//! rustpush configuration that keeps Mac validation behind LiteBubbles' provider.
//!
//! The provider creates only the validation artifact.  It never exposes the
//! foreign library, FairPlay keys, or validation implementation to GTK or
//! D-Bus.  The remaining FairPlay device-activation signer is deliberately a
//! separate rustpush boundary and still reports unavailable in public builds.

use std::{collections::HashMap, sync::Arc};

use litebubbles_validation_provider::{
    HardwareConfig as ProviderHardwareConfig, OpenBubblesValidationProvider, ValidationError,
    ValidationProvider, ValidationRequest,
};
use plist::{Data, Value};
use serde::{Deserialize, Serialize};

use crate::{MacHardwareConfig, MacHardwareInput};

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct SessionInfoRequest {
    session_info_request: Data,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct SessionInfoResponse {
    session_info: Data,
}

#[derive(Deserialize)]
struct CertsResponse {
    cert: Data,
}

/// A rustpush [`OSConfig`](rustpush::OSConfig) backed by the separately
/// installed OpenBubbles validation component.
pub struct ValidationBackedMacOsConfig {
    upstream: rustpush::macos::MacOSConfig,
    hardware: MacHardwareInput,
    provider: Arc<dyn ValidationProvider>,
}

impl ValidationBackedMacOsConfig {
    /// Build a configuration from manually entered genuine-Mac hardware data.
    pub fn new(hardware: MacHardwareInput, provider: Arc<dyn ValidationProvider>) -> Self {
        let software = hardware.software();
        let inner = hardware.hardware();
        let upstream = rustpush::macos::MacOSConfig {
            inner: rustpush::macos::HardwareConfig {
                product_name: inner.product_name().to_owned(),
                io_mac_address: *inner.io_mac_address(),
                platform_serial_number: inner.platform_serial_number().to_owned(),
                platform_uuid: inner.platform_uuid().to_owned(),
                root_disk_uuid: inner.root_disk_uuid().to_owned(),
                board_id: inner.board_id().to_owned(),
                os_build_num: inner.os_build_num().to_owned(),
                platform_serial_number_enc: inner.platform_serial_number_enc().to_vec(),
                platform_uuid_enc: inner.platform_uuid_enc().to_vec(),
                root_disk_uuid_enc: inner.root_disk_uuid_enc().to_vec(),
                rom: inner.rom().to_vec(),
                rom_enc: inner.rom_enc().to_vec(),
                mlb: inner.mlb().to_owned(),
                mlb_enc: inner.mlb_enc().to_vec(),
            },
            version: software.version().to_owned(),
            protocol_version: software.protocol_version(),
            device_id: software.device_id().to_owned(),
            icloud_ua: software.icloud_ua().to_owned(),
            aoskit_version: software.aoskit_version().to_owned(),
            udid: Some(software.device_id().to_owned()),
        };

        Self {
            upstream,
            hardware,
            provider,
        }
    }

    /// Build the production configuration using the component installed by
    /// `litebubbles-validation-component`.
    pub fn from_default_provider(hardware: MacHardwareInput) -> Result<Self, ValidationError> {
        let provider = OpenBubblesValidationProvider::from_default_paths()?;
        Ok(Self::new(hardware, Arc::new(provider)))
    }

    pub fn hardware(&self) -> &MacHardwareInput {
        &self.hardware
    }

    pub fn provider_is_available(&self) -> bool {
        self.provider.is_available()
    }

    // rustpush owns this error type as part of its OSConfig contract. The
    // clippy size warning is therefore an external API boundary, not an
    // allocation introduced by the provider.
    #[allow(clippy::result_large_err)]
    async fn generate_validation_data_with_provider(&self) -> Result<Vec<u8>, rustpush::PushError> {
        if !self.provider.is_available() {
            return Err(provider_error(ValidationError::MissingComponent));
        }

        let cert_url = rustpush::get_bag(rustpush::IDS_BAG, "id-validation-cert")
            .await?
            .into_string()
            .ok_or(rustpush::PushError::BagKeyNotFound)?;
        let cert_response = rustpush::REQWEST.get(cert_url).send().await?;
        let certs: CertsResponse = plist::from_bytes(&cert_response.bytes().await?)?;

        let session = self
            .provider
            .begin(ValidationRequest {
                hardware_config: provider_hardware(self.hardware.hardware()),
                cert_data: certs.cert.into(),
            })
            .map_err(provider_error)?;

        let request = rustpush::plist_to_buf(&SessionInfoRequest {
            session_info_request: session.session_info().to_vec().into(),
        })?;
        let initialize_url = rustpush::get_bag(rustpush::IDS_BAG, "id-initialize-validation")
            .await?
            .into_string()
            .ok_or(rustpush::PushError::BagKeyNotFound)?;
        let response = rustpush::REQWEST
            .post(initialize_url)
            .body(request)
            .send()
            .await?;
        let response: SessionInfoResponse = plist::from_bytes(&response.bytes().await?)?;

        session
            .finish(response.session_info.into())
            .map(|data| data.into_bytes())
            .map_err(provider_error)
    }
}

#[async_trait::async_trait]
impl rustpush::OSConfig for ValidationBackedMacOsConfig {
    fn build_activation_info(&self, csr: Vec<u8>) -> rustpush::ActivationInfo {
        self.upstream.build_activation_info(csr)
    }

    fn get_activation_device(&self) -> String {
        self.upstream.get_activation_device()
    }

    async fn generate_validation_data(&self) -> Result<Vec<u8>, rustpush::PushError> {
        self.generate_validation_data_with_provider().await
    }

    fn get_protocol_version(&self) -> u32 {
        self.upstream.get_protocol_version()
    }

    fn get_register_meta(&self) -> rustpush::RegisterMeta {
        self.upstream.get_register_meta()
    }

    fn get_normal_ua(&self, item: &str) -> String {
        self.upstream.get_normal_ua(item)
    }

    fn get_mme_clientinfo(&self, for_item: &str) -> String {
        self.upstream.get_mme_clientinfo(for_item)
    }

    fn get_version_ua(&self) -> String {
        self.upstream.get_version_ua()
    }

    fn get_device_name(&self) -> String {
        self.upstream.get_device_name()
    }

    fn get_device_uuid(&self) -> String {
        self.upstream.get_device_uuid()
    }

    fn get_private_data(&self) -> plist::Dictionary {
        self.upstream.get_private_data()
    }

    fn get_debug_meta(&self) -> rustpush::DebugMeta {
        self.upstream.get_debug_meta()
    }

    fn get_login_url(&self) -> &'static str {
        self.upstream.get_login_url()
    }

    fn get_serial_number(&self) -> String {
        self.upstream.get_serial_number()
    }

    fn get_gsa_hardware_headers(&self) -> HashMap<String, String> {
        self.upstream.get_gsa_hardware_headers()
    }

    fn get_aoskit_version(&self) -> String {
        self.upstream.get_aoskit_version()
    }

    fn get_udid(&self) -> String {
        self.upstream.get_udid()
    }
}

fn provider_hardware(config: &MacHardwareConfig) -> ProviderHardwareConfig {
    ProviderHardwareConfig {
        product_name: config.product_name().to_owned(),
        io_mac_address: *config.io_mac_address(),
        platform_serial_number: config.platform_serial_number().to_owned(),
        platform_uuid: config.platform_uuid().to_owned(),
        root_disk_uuid: config.root_disk_uuid().to_owned(),
        board_id: config.board_id().to_owned(),
        os_build_num: config.os_build_num().to_owned(),
        platform_serial_number_enc: config.platform_serial_number_enc().to_vec(),
        platform_uuid_enc: config.platform_uuid_enc().to_vec(),
        root_disk_uuid_enc: config.root_disk_uuid_enc().to_vec(),
        rom: config.rom().to_vec(),
        rom_enc: config.rom_enc().to_vec(),
        mlb: config.mlb().to_owned(),
        mlb_enc: config.mlb_enc().to_vec(),
    }
}

fn provider_error(error: ValidationError) -> rustpush::PushError {
    rustpush::PushError::FilePackageError(format!("Apple validation provider unavailable: {error}"))
}

/// Resolve the installed production provider at the backend boundary. The
/// caller can turn `MissingComponent` into setup UI without exposing paths or
/// provider internals over D-Bus.
pub fn production_validation_provider() -> Result<Arc<dyn ValidationProvider>, ValidationError> {
    Ok(Arc::new(
        OpenBubblesValidationProvider::from_default_paths()?
    ))
}

/// Keeps the private provider output out of ordinary debug formatting.
pub fn redacted_validation_status(provider: &dyn ValidationProvider) -> Value {
    Value::String(if provider.is_available() {
        "available".to_owned()
    } else {
        "unavailable".to_owned()
    })
}
