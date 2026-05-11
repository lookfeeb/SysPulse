//! Safe wrapper over `GetIfTable2` to enumerate Windows network interfaces.
//!
//! Returns one row per interface with byte counters and metadata. The byte
//! counters are 64-bit cumulative since interface bring-up.

use crate::error::Result;
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2,
};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

// Hard-coded IANA ifType numbers (RFC 1213 / IF-MIB).
const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const IF_TYPE_IEEE80211: u32 = 71;
const IF_TYPE_TUNNEL: u32 = 131;
const IF_TYPE_PPP: u32 = 23;

#[derive(Debug, Clone)]
pub struct IfRow {
    pub luid: u64,
    pub index: u32,
    pub name: String,
    pub description: String,
    pub if_type: u32,
    pub is_up: bool,
    pub is_loopback: bool,
    pub is_physical: bool,
    pub is_tunnel: bool,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub mtu: u32,
}

pub fn list_interfaces() -> Result<Vec<IfRow>> {
    unsafe {
        let mut table_ptr: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        let r = GetIfTable2(&mut table_ptr);
        if r.0 != 0 || table_ptr.is_null() {
            return Err(crate::error::AppError::Collect {
                what: "if_table",
                msg: format!("GetIfTable2 failed: code {}", r.0),
            });
        }

        let table = &*table_ptr;
        let count = table.NumEntries as usize;
        let rows_ptr = table.Table.as_ptr();
        let mut out = Vec::with_capacity(count);

        for i in 0..count {
            let row: &MIB_IF_ROW2 = &*rows_ptr.add(i);
            let alias = utf16_to_string(&row.Alias);
            let desc = utf16_to_string(&row.Description);
            let is_up = row.OperStatus == IfOperStatusUp;
            let is_loopback = row.Type == IF_TYPE_SOFTWARE_LOOPBACK;
            let is_physical = row.Type == IF_TYPE_ETHERNET_CSMACD || row.Type == IF_TYPE_IEEE80211;
            let is_tunnel = row.Type == IF_TYPE_TUNNEL || row.Type == IF_TYPE_PPP;

            out.push(IfRow {
                luid: row.InterfaceLuid.Value,
                index: row.InterfaceIndex,
                name: alias,
                description: desc,
                if_type: row.Type,
                is_up,
                is_loopback,
                is_physical,
                is_tunnel,
                bytes_in: row.InOctets,
                bytes_out: row.OutOctets,
                mtu: row.Mtu,
            });
        }

        FreeMibTable(table_ptr as *const _ as _);
        Ok(out)
    }
}

fn utf16_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}
