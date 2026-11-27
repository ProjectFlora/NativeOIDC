//! Encodes the Windows version resource of `native_oidc.dll` as a compiled
//! resource file (`.res`), without the Windows SDK resource compiler.
//!
//! `build.rs` writes the result to `OUT_DIR` and passes it to the MSVC
//! linker, which accepts `.res` files as inputs. The layout follows the
//! `VS_VERSIONINFO`, `StringFileInfo`, `StringTable`, `String`,
//! `VarFileInfo`, and `Var` structures of the Windows SDK: each node is
//! `wLength`, `wValueLength`, `wType`, a NUL-terminated UTF-16 key, padding
//! to a 32-bit boundary, the value, and the children, each starting on a
//! 32-bit boundary. A `.res` file starts with an empty resource entry,
//! followed by one 32-byte header and the data for each resource.
//!
//! SignPath enforces the product name and product version of every signed
//! binary (Issue #258), so the strings come from `native_oidc_strings`, and
//! the version from the crate version.

/// `RT_VERSION`, the resource type of a version resource.
pub const RT_VERSION: u16 = 16;
/// `VS_VERSION_INFO`, the resource name (ordinal 1) of the version resource.
pub const VERSION_RESOURCE_NAME: u16 = 1;
/// U.S. English, the language of the string table.
pub const LANGUAGE_EN_US: u16 = 0x0409;
/// UTF-16 (code page 1200), the code page of the string table.
pub const CODE_PAGE_UTF16: u16 = 0x04B0;
/// `VS_FFI_SIGNATURE`, the first field of `VS_FIXEDFILEINFO`.
pub const FIXED_FILE_INFO_SIGNATURE: u32 = 0xFEEF_04BD;
/// `VFT_DLL`, the file type of a dynamic-link library.
pub const FILE_TYPE_DLL: u32 = 2;
/// `MOVEABLE | PURE`, the memory flags the resource compiler sets on a
/// version resource.
pub const VERSION_MEMORY_FLAGS: u16 = 0x0030;

const FIXED_FILE_INFO_LENGTH: usize = 52;
const VALUE_BINARY: u16 = 0;
const VALUE_TEXT: u16 = 1;

/// The string values of the version resource for a crate version such as
/// `0.9.0`, in the order they are written.
pub fn native_oidc_strings(crate_version: &str) -> Vec<(&'static str, String)> {
    vec![
        ("CompanyName", "ProjectFlora".to_owned()),
        (
            "FileDescription",
            "NativeOIDC OAuth 2.0 and OpenID Connect client library".to_owned(),
        ),
        ("FileVersion", crate_version.to_owned()),
        ("InternalName", "native_oidc".to_owned()),
        (
            "LegalCopyright",
            "Copyright ProjectFlora. Licensed under the Apache License, Version 2.0.".to_owned(),
        ),
        ("OriginalFilename", "native_oidc.dll".to_owned()),
        ("ProductName", "NativeOIDC".to_owned()),
        ("ProductVersion", crate_version.to_owned()),
    ]
}

/// Encodes a `.res` file holding one version resource for a DLL, with the
/// same numeric file and product version, `version` as
/// `[major, minor, patch, build]`.
pub fn encode_res(version: [u16; 4], strings: &[(&str, String)]) -> Vec<u8> {
    let data = version_info(version, strings);
    let mut res = Vec::new();
    // The empty entry that starts every .res file: no data, type 0, name 0.
    push_resource_header(&mut res, 0, 0, 0, 0, 0);
    push_resource_header(
        &mut res,
        u32::try_from(data.len()).expect("version resource exceeds 4 GiB"),
        RT_VERSION,
        VERSION_RESOURCE_NAME,
        VERSION_MEMORY_FLAGS,
        LANGUAGE_EN_US,
    );
    res.extend_from_slice(&data);
    pad_to_dword(&mut res);
    res
}

/// The `VS_VERSIONINFO` tree, the data of the version resource.
pub fn version_info(version: [u16; 4], strings: &[(&str, String)]) -> Vec<u8> {
    let string_nodes: Vec<Vec<u8>> = strings
        .iter()
        .map(|(key, value)| {
            let text = utf16_nul(value);
            // A String node's wValueLength counts UTF-16 code units,
            // including the terminating NUL.
            let units = u16::try_from(text.len() / 2).expect("version string too long");
            node(key, VALUE_TEXT, &text, units, &[])
        })
        .collect();
    let table_key = format!("{LANGUAGE_EN_US:04X}{CODE_PAGE_UTF16:04X}");
    let string_table = node(&table_key, VALUE_TEXT, &[], 0, &string_nodes);
    let string_file_info = node("StringFileInfo", VALUE_TEXT, &[], 0, &[string_table]);

    let mut translation = Vec::new();
    translation.extend_from_slice(&LANGUAGE_EN_US.to_le_bytes());
    translation.extend_from_slice(&CODE_PAGE_UTF16.to_le_bytes());
    let var = node("Translation", VALUE_BINARY, &translation, 4, &[]);
    let var_file_info = node("VarFileInfo", VALUE_TEXT, &[], 0, &[var]);

    let fixed = fixed_file_info(version);
    node(
        "VS_VERSION_INFO",
        VALUE_BINARY,
        &fixed,
        FIXED_FILE_INFO_LENGTH as u16,
        &[string_file_info, var_file_info],
    )
}

/// `VS_FIXEDFILEINFO` for a Windows NT DLL, without file flags or date.
fn fixed_file_info(version: [u16; 4]) -> Vec<u8> {
    let [major, minor, patch, build] = version.map(u32::from);
    let version_ms = (major << 16) | minor;
    let version_ls = (patch << 16) | build;
    let fields: [u32; 13] = [
        FIXED_FILE_INFO_SIGNATURE,
        0x0001_0000, // dwStrucVersion 1.0
        version_ms,  // dwFileVersionMS
        version_ls,  // dwFileVersionLS
        version_ms,  // dwProductVersionMS
        version_ls,  // dwProductVersionLS
        0x0000_003F, // dwFileFlagsMask: VS_FFI_FILEFLAGSMASK
        0,           // dwFileFlags: a release build
        0x0004_0004, // dwFileOS: VOS_NT_WINDOWS32
        FILE_TYPE_DLL,
        0, // dwFileSubtype
        0, // dwFileDateMS
        0, // dwFileDateLS
    ];
    let bytes: Vec<u8> = fields
        .iter()
        .flat_map(|field| field.to_le_bytes())
        .collect();
    debug_assert_eq!(bytes.len(), FIXED_FILE_INFO_LENGTH);
    bytes
}

/// One version-information node. `wLength` covers the node and its
/// children, but not the padding that may follow the node.
fn node(
    key: &str,
    value_type: u16,
    value: &[u8],
    value_length: u16,
    children: &[Vec<u8>],
) -> Vec<u8> {
    let mut out = vec![0; 6];
    out.extend_from_slice(&utf16_nul(key));
    pad_to_dword(&mut out);
    out.extend_from_slice(value);
    for child in children {
        pad_to_dword(&mut out);
        out.extend_from_slice(child);
    }
    let length = u16::try_from(out.len()).expect("version resource node exceeds 64 KiB");
    out[0..2].copy_from_slice(&length.to_le_bytes());
    out[2..4].copy_from_slice(&value_length.to_le_bytes());
    out[4..6].copy_from_slice(&value_type.to_le_bytes());
    out
}

/// A 32-byte `RESOURCEHEADER` with ordinal type and name.
fn push_resource_header(
    out: &mut Vec<u8>,
    data_size: u32,
    resource_type: u16,
    name: u16,
    memory_flags: u16,
    language: u16,
) {
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    out.extend_from_slice(&0xFFFFu16.to_le_bytes()); // TYPE: an ordinal follows
    out.extend_from_slice(&resource_type.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes()); // NAME: an ordinal follows
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&memory_flags.to_le_bytes());
    out.extend_from_slice(&language.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
}

fn utf16_nul(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn pad_to_dword(out: &mut Vec<u8>) {
    while out.len() % 4 != 0 {
        out.push(0);
    }
}
