//! COM apartment lifetime.

use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, COINIT_MULTITHREADED, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};

use crate::error::{Context, Result};

/// Initializes COM on the current thread and uninitializes it on drop.
/// Must be dropped on the thread that created it.
#[derive(Debug)]
pub struct ComApartment {
    ole: bool,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl ComApartment {
    /// Single-threaded apartment with OLE (drag and drop, clipboard).
    /// Used by the UI thread.
    pub fn ole_sta() -> Result<ComApartment> {
        // SAFETY: OleInitialize only requires a NULL reserved pointer.
        // https://learn.microsoft.com/windows/win32/api/ole2/nf-ole2-oleinitialize
        unsafe { OleInitialize(None) }.ctx("OleInitialize")?;
        Ok(ComApartment {
            ole: true,
            _not_send: std::marker::PhantomData,
        })
    }

    /// Single-threaded apartment without OLE.
    pub fn sta() -> Result<ComApartment> {
        // SAFETY: plain flag values, reserved pointer is None.
        // https://learn.microsoft.com/windows/win32/api/combaseapi/nf-combaseapi-coinitializeex
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
            .ok()
            .ctx("CoInitializeEx(STA)")?;
        Ok(ComApartment {
            ole: false,
            _not_send: std::marker::PhantomData,
        })
    }

    /// Multithreaded apartment, used by background workers.
    pub fn mta() -> Result<ComApartment> {
        // SAFETY: as above.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .ctx("CoInitializeEx(MTA)")?;
        Ok(ComApartment {
            ole: false,
            _not_send: std::marker::PhantomData,
        })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: balanced with the successful initialization of `new`, on
        // the same thread (the type is !Send).
        unsafe {
            if self.ole {
                OleUninitialize();
            } else {
                CoUninitialize();
            }
        }
    }
}
