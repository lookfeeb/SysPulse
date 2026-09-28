use super::*;

#[derive(Clone, Copy)]
pub(super) struct ProtoField<'a> {
    pub(super) number: u64,
    pub(super) wire_type: u8,
    pub(super) raw: &'a [u8],
    pub(super) value: Option<&'a [u8]>,
}

pub(super) fn read_proto_varint(bytes: &[u8], index: &mut usize) -> anyhow::Result<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes
            .get(*index)
            .ok_or_else(|| anyhow::anyhow!("截断的 protobuf varint"))?;
        *index += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(anyhow::anyhow!("protobuf varint 过长"))
}

pub(super) fn protobuf_fields(bytes: &[u8]) -> anyhow::Result<Vec<ProtoField<'_>>> {
    let mut fields = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let start = index;
        let key = read_proto_varint(bytes, &mut index)?;
        let number = key >> 3;
        let wire_type = (key & 0x07) as u8;
        let value = match wire_type {
            0 => {
                read_proto_varint(bytes, &mut index)?;
                None
            }
            1 => {
                index = index
                    .checked_add(8)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("截断的 protobuf fixed64"))?;
                None
            }
            2 => {
                let len = usize::try_from(read_proto_varint(bytes, &mut index)?)
                    .map_err(|_| anyhow::anyhow!("protobuf 字段长度过大"))?;
                let end = index
                    .checked_add(len)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("截断的 protobuf bytes 字段"))?;
                let value = &bytes[index..end];
                index = end;
                Some(value)
            }
            5 => {
                index = index
                    .checked_add(4)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("截断的 protobuf fixed32"))?;
                None
            }
            _ => return Err(anyhow::anyhow!("不支持的 protobuf wire type: {wire_type}")),
        };
        fields.push(ProtoField {
            number,
            wire_type,
            raw: &bytes[start..index],
            value,
        });
    }
    Ok(fields)
}

pub(super) fn protobuf_map_entry_id(entry: &[u8]) -> Option<String> {
    protobuf_fields(entry)
        .ok()?
        .into_iter()
        .find(|field| field.number == 1 && field.wire_type == 2)
        .and_then(|field| field.value)
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(extract_uuid)
}

pub(super) fn protobuf_map_entries_result(bytes: &[u8]) -> anyhow::Result<Vec<(&[u8], String)>> {
    Ok(protobuf_fields(bytes)?
        .into_iter()
        .filter(|field| field.number == 1 && field.wire_type == 2)
        .filter_map(|field| {
            let value = field.value?;
            Some((value, protobuf_map_entry_id(value)?))
        })
        .collect())
}

pub(super) fn protobuf_string_map_entries_result(
    bytes: &[u8],
) -> anyhow::Result<Vec<(String, &[u8])>> {
    let mut out = Vec::new();
    for field in protobuf_fields(bytes)? {
        if field.number != 1 || field.wire_type != 2 {
            continue;
        }
        let Some(entry) = field.value else {
            continue;
        };
        let fields = protobuf_fields(entry)?;
        let key = fields
            .iter()
            .find(|field| field.number == 1 && field.wire_type == 2)
            .and_then(|field| field.value)
            .and_then(|value| std::str::from_utf8(value).ok())
            .map(str::to_string);
        let value = fields
            .iter()
            .find(|field| field.number == 2 && field.wire_type == 2)
            .and_then(|field| field.value);
        if let (Some(key), Some(value)) = (key, value) {
            out.push((key, value));
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(super) fn protobuf_map_entries(bytes: &[u8]) -> Vec<(&[u8], String)> {
    protobuf_map_entries_result(bytes).unwrap_or_default()
}

pub(super) fn remove_protobuf_map_entries(
    bytes: &[u8],
    ids: &HashSet<String>,
) -> anyhow::Result<(Vec<u8>, usize)> {
    let fields = protobuf_fields(bytes)?;
    let mut output = Vec::with_capacity(bytes.len());
    let mut removed = 0usize;
    for field in fields {
        let remove = field.number == 1
            && field.wire_type == 2
            && field
                .value
                .and_then(protobuf_map_entry_id)
                .is_some_and(|id| ids.contains(&id));
        if remove {
            removed += 1;
        } else {
            output.extend_from_slice(field.raw);
        }
    }
    Ok((output, removed))
}

pub(super) fn remove_protobuf_string_map_entries(
    bytes: &[u8],
    ids: &HashSet<String>,
) -> anyhow::Result<(Vec<u8>, usize)> {
    if ids.is_empty() {
        return Ok((bytes.to_vec(), 0));
    }
    let fields = protobuf_fields(bytes)?;
    let mut output = Vec::with_capacity(bytes.len());
    let mut removed = 0usize;
    for field in fields {
        let remove = field.number == 1
            && field.wire_type == 2
            && field.value.is_some_and(|entry| {
                protobuf_fields(entry)
                    .ok()
                    .and_then(|fields| {
                        fields
                            .into_iter()
                            .find(|field| field.number == 1 && field.wire_type == 2)
                    })
                    .and_then(|field| field.value)
                    .and_then(|value| std::str::from_utf8(value).ok())
                    .and_then(extract_uuid)
                    .is_some_and(|id| ids.contains(&id))
            });
        if remove {
            removed += 1;
        } else {
            output.extend_from_slice(field.raw);
        }
    }
    Ok((output, removed))
}
