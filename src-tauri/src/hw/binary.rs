//! Bounded SPW1/SPW2 decoding; mirrors SnapshotBinary.cs. No JSON intermediate tree.
use super::snapshot::*;
use std::io::{Cursor, Read};
type Result<T> = std::result::Result<T, String>;
struct Decoder<'a>(Cursor<&'a [u8]>);
impl<'a> Decoder<'a> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut value = [0; N];
        self.0
            .read_exact(&mut value)
            .map_err(|_| "Truncated hardware snapshot")?;
        Ok(value)
    }
    fn flag(&mut self) -> Result<bool> {
        match self.bytes::<1>()?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("Invalid presence flag".into()),
        }
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }
    fn f64(&mut self) -> Result<f64> {
        let n = f64::from_le_bytes(self.bytes()?);
        if n.is_finite() {
            Ok(n)
        } else {
            Err("Non-finite hardware reading".into())
        }
    }
    fn optional<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        if self.flag()? {
            f(self).map(Some)
        } else {
            Ok(None)
        }
    }
    fn number(&mut self) -> Result<Option<f64>> {
        self.optional(Self::f64)
    }
    fn text(&mut self) -> Result<String> {
        let size = self.u32()? as usize;
        let pos = self.0.position() as usize;
        let end = pos.checked_add(size).ok_or("String length overflow")?;
        let bytes = self.0.get_ref().get(pos..end).ok_or("Truncated string")?;
        let value = std::str::from_utf8(bytes)
            .map_err(|_| "Invalid UTF-8")?
            .to_owned();
        self.0.set_position(end as u64);
        Ok(value)
    }
    fn list<T>(&mut self, mut f: impl FnMut(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        let count = self.u32()? as usize;
        if count > 16384
            || count
                > self
                    .0
                    .get_ref()
                    .len()
                    .saturating_sub(self.0.position() as usize)
        {
            return Err("Hardware list exceeds bounds".into());
        }
        (0..count).map(|_| f(self)).collect()
    }
    fn named(&mut self) -> Result<NamedValue> {
        Ok(NamedValue {
            name: self.text()?,
            value: self.f64()?,
            identifier: self.text()?,
        })
    }
}
pub fn decode(data: &[u8]) -> Result<HwSnapshot> {
    if data.len() > 4 * 1024 * 1024 {
        return Err("Hardware frame exceeds bounds".into());
    }
    let mut d = Decoder(Cursor::new(data));
    let version = d.bytes::<4>()?;
    if version != *b"SPW1" && version != *b"SPW2" {
        return Err("Unsupported hardware ABI version".into());
    }
    let timestamp_ms = i64::from_le_bytes(d.bytes()?);
    let sample_age_ms = d.u64()?;
    let storage_age_ms = d.u64()?;
    let cpu = d.optional(|d| {
        Ok(CpuHw {
            name: d.text()?,
            package_temp_c: d.number()?,
            per_core_temps_c: d.list(Decoder::number)?,
            per_core_usage: d.list(Decoder::number)?,
            total_usage: d.number()?,
            frequency_mhz: d.number()?,
            power_w: d.number()?,
        })
    })?;
    let gpus = d.list(|d| {
        Ok(GpuHw {
            index: d.u32()?,
            name: d.text()?,
            vendor: d.text()?,
            usage_percent: d.number()?,
            mem_used_mb: d.number()?,
            mem_total_mb: d.number()?,
            temp_c: d.number()?,
            power_w: d.number()?,
            fan_rpm: d.number()?,
            fan_pwm: d.number()?,
        })
    })?;
    let memory = d.optional(|d| {
        Ok(MemoryHw {
            total_bytes: d.u64()?,
            used_bytes: d.u64()?,
            used_percent: d.f64()?,
            swap_total_bytes: d.u64()?,
            swap_used_bytes: d.u64()?,
            modules: d.list(|d| {
                Ok(MemoryModule {
                    slot: d.text()?,
                    capacity_bytes: d.u64()?,
                    speed_mtps: d.number()?,
                    manufacturer: d.text()?,
                    part_number: d.text()?,
                })
            })?,
            frequency_mhz: d.number()?,
            channels: d.optional(Decoder::u32)?,
        })
    })?;
    let disks = d.list(|d| {
        Ok(DiskHw {
            index: d.u32()?,
            model: d.text()?,
            bus: d.text()?,
            temp_c: d.number()?,
            health: d.text()?,
            read_bytes_per_sec: d.number()?,
            write_bytes_per_sec: d.number()?,
            total_bytes: d.u64()?,
            used_bytes: d.optional(Decoder::u64)?,
            identifier: d.text()?,
        })
    })?;
    let motherboard = d.optional(|d| {
        Ok(MotherboardHw {
            vendor: d.text()?,
            model: d.text()?,
            super_io: d.optional(Decoder::text)?,
            temperatures_c: d.list(Decoder::named)?,
            voltages_v: d.list(Decoder::named)?,
            interfaces_age_ms: d.u64()?,
            interfaces: d.list(|d| {
                Ok(BoardInterface {
                    id: d.text()?,
                    name: d.text()?,
                    kind: d.text()?,
                    connector: d.optional(Decoder::text)?,
                    supported_speed: d.optional(Decoder::text)?,
                    current_speed: d.optional(Decoder::text)?,
                    status: d.text()?,
                    device: d.optional(Decoder::text)?,
                    source: d.text()?,
                    usb: if version == *b"SPW2" {
                        d.optional(|d| {
                            let logical_port_count = d.u32()?;
                            if !(1..=256).contains(&logical_port_count) {
                                return Err("Invalid USB logical port count".into());
                            }
                            Ok(UsbInterfaceDetails {
                                logical_port_count,
                                user_connectable: d.optional(Decoder::flag)?,
                                maximum_speed_source: d.optional(Decoder::text)?,
                                device_capability: d.optional(Decoder::text)?,
                                device_capability_source: d.optional(Decoder::text)?,
                                topology_note: d.optional(Decoder::text)?,
                            })
                        })?
                    } else {
                        None
                    },
                })
            })?,
        })
    })?;
    let fans = d.list(|d| {
        Ok(FanHw {
            id: d.text()?,
            name: d.text()?,
            rpm: d.number()?,
            pwm_percent: d.number()?,
            controllable: d.flag()?,
        })
    })?;
    if d.0.position() as usize != data.len() {
        return Err("Unexpected trailing hardware data".into());
    }
    Ok(HwSnapshot {
        received_at: Some(std::time::Instant::now()),
        timestamp_ms,
        sample_age_ms,
        storage_age_ms,
        cpu,
        gpus,
        memory,
        disks,
        motherboard,
        fans,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_board_frames_and_optional_usb_metadata_remain_compatible() {
        // SPW1 fixture: no sensor data, one otherwise empty board interface.
        let mut frame = b"SPW1".to_vec();
        frame.extend([0; 34]); // timestamp/ages, CPU, GPU count, memory, disk count
        frame.push(1); // motherboard present
        frame.extend([0; 25]); // vendor/model, SuperIO, temperatures, voltages, age
        frame.extend(1u32.to_le_bytes()); // one interface
        frame.extend([0; 19]); // id/name/kind, connector/max/current, status
        frame.push(0); // device absent
        frame.extend(3u32.to_le_bytes());
        frame.extend(b"usb");
        let metadata_offset = frame.len();
        frame.extend([0; 4]); // no fans
        let old = decode(&frame).unwrap();
        let port = &old.motherboard.unwrap().interfaces[0];
        assert_eq!(port.source, "usb");
        assert!(port.usb.is_none());

        frame[..4].copy_from_slice(b"SPW2");
        frame.insert(metadata_offset, 0); // optional metadata absent
        assert!(decode(&frame).unwrap().motherboard.unwrap().interfaces[0]
            .usb
            .is_none());
        for count in [0u32, 257, u32::MAX] {
            let mut invalid = frame[..metadata_offset].to_vec();
            invalid.push(1);
            invalid.extend(count.to_le_bytes());
            invalid.extend([0; 10]); // optional metadata fields and fan count
            assert!(decode(&invalid).unwrap_err().contains("logical port count"));
        }
    }
    #[test]
    fn rejects_malformed_or_unsupported_frames_without_allocating_huge_lists() {
        for bytes in [&b""[..], &b"SPW2"[..], &b"SPW1"[..], &[255u8; 128][..]] {
            assert!(decode(bytes).is_err());
        }
        let mut d = Decoder(Cursor::new(&[255u8; 4][..]));
        assert!(d.list(Decoder::u32).is_err());
        let mut d = Decoder(Cursor::new(&[2u8][..]));
        assert!(d.flag().is_err());
    }
    #[test]
    fn reads_csharp_fixture_when_requested() {
        if let Ok(file) = std::env::var("HW_ABI_FIXTURE") {
            let frame = std::fs::read(file).unwrap();
            let value = decode(&frame).unwrap();
            assert_eq!(value.timestamp_ms, 123);
            assert_eq!(value.cpu.as_ref().unwrap().package_temp_c, Some(0.0));
            assert_eq!(value.memory.as_ref().unwrap().total_bytes, 1099511627776);
            assert_eq!(value.fans[0].rpm, Some(0.0));
            assert_eq!(
                value.motherboard.as_ref().unwrap().interfaces[0].name,
                "USB 接口"
            );
            let usb = value.motherboard.as_ref().unwrap().interfaces[0]
                .usb
                .as_ref()
                .unwrap();
            assert_eq!(usb.logical_port_count, 2);
            assert_eq!(usb.user_connectable, Some(true));
            assert_eq!(usb.device_capability.as_deref(), Some("5 Gb/s"));
            assert_eq!(usb.maximum_speed_source.as_deref(), Some("测试来源"));
            for n in 0..frame.len() {
                assert!(decode(&frame[..n]).is_err());
            }
            let mut trailing = frame;
            trailing.push(0);
            assert!(decode(&trailing).is_err());
        }
    }
    #[test]
    #[ignore = "reads a snapshot produced by the native real-hardware probe"]
    fn real_native_snapshot_decodes() {
        let frame = std::fs::read(std::env::var("HW_NATIVE_FRAME").unwrap()).unwrap();
        let snapshot = decode(&frame).unwrap();
        println!("{}", serde_json::to_string_pretty(&snapshot).unwrap());
        assert!(snapshot.memory.as_ref().unwrap().total_bytes > 0);
        assert!(snapshot
            .motherboard
            .as_ref()
            .is_some_and(|m| !m.interfaces.is_empty()));
    }
}
