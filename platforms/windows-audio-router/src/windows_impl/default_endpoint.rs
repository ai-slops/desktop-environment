// windows::core::implement generates these casts and inline annotations.
#![allow(clippy::ref_as_ptr, clippy::inline_always)]

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE, EDataFlow, ERole, IMMNotificationClient, IMMNotificationClient_Impl, eConsole,
    eRender,
};
use windows::core::{PCWSTR, implement};

// The callback only advances an epoch: no COM calls, locks or stream teardown
// occur on the audio service notification thread. All reconnection runs here.
#[implement(IMMNotificationClient)]
pub(super) struct DefaultEndpointNotification {
    pub(super) generation: Arc<AtomicU64>,
}

impl IMMNotificationClient_Impl for DefaultEndpointNotification_Impl {
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}
