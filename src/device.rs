use ibverbs::{Context, DeviceList};
use std::io::{Error, ErrorKind, Result};

/// Opens an RDMA device context, either the one matching `name` or the first one available.
pub fn open(name: Option<&str>) -> Result<Context> {
    let devices: DeviceList = ibverbs::devices()?;

    let device = match name {
        Some(name) => devices
            .iter()
            .find(|d| {
                d.name()
                    .map(|n| n.to_string_lossy().as_ref() == name)
                    .unwrap_or(false)
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::NotFound,
                    format!("no RDMA device named '{name}' found"),
                )
            })?,
        None => devices
            .iter()
            .next()
            .ok_or(Error::new(ErrorKind::NotFound, "no RDMA device available"))?,
    };

    device.open()
}
