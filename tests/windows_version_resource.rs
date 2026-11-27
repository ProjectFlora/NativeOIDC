//! Checks the `.res` encoder that `build.rs` links into `native_oidc.dll`
//! (Issue #258). The Windows CI jobs also read the built DLL's version
//! resource back (`scripts/package-windows.ps1`); this test checks the
//! encoded layout on every host.

#[path = "../build_support/windows_version_resource.rs"]
mod windows_version_resource;

use windows_version_resource::{
    encode_res, native_oidc_strings, version_info, CODE_PAGE_UTF16, FILE_TYPE_DLL,
    FIXED_FILE_INFO_SIGNATURE, LANGUAGE_EN_US, RT_VERSION, VERSION_MEMORY_FLAGS,
    VERSION_RESOURCE_NAME,
};

#[derive(Debug)]
struct Node {
    value_type: u16,
    key: String,
    value: Vec<u8>,
    children: Vec<Node>,
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn align(offset: usize) -> usize {
    (offset + 3) & !3
}

fn utf16_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .collect();
    String::from_utf16(&units).unwrap()
}

/// Parses one node starting at `start` (a 32-bit boundary), checking that
/// it and its children stay inside `end`.
fn parse_node(bytes: &[u8], start: usize, end: usize) -> Node {
    assert_eq!(start % 4, 0, "node at {start} is not DWORD-aligned");
    let length = usize::from(u16_at(bytes, start));
    let value_length = usize::from(u16_at(bytes, start + 2));
    let value_type = u16_at(bytes, start + 4);
    let node_end = start + length;
    assert!(
        node_end <= end,
        "node at {start} ends at {node_end}, past {end}"
    );
    assert!(value_type <= 1, "node at {start} has wType {value_type}");

    let mut key_end = start + 6;
    while u16_at(bytes, key_end) != 0 {
        key_end += 2;
    }
    let key = utf16_text(&bytes[start + 6..key_end]);

    let value_start = align(key_end + 2);
    // Text values count UTF-16 code units; binary values count bytes.
    let value_size = if value_type == 1 {
        value_length * 2
    } else {
        value_length
    };
    let value = bytes[value_start..value_start + value_size].to_vec();
    if value_type == 1 && value_size > 0 {
        assert_eq!(
            &value[value_size - 2..],
            &[0, 0],
            "text value of {key} is not NUL-terminated"
        );
    }

    let mut children = Vec::new();
    let mut child_start = align(value_start + value_size);
    while child_start < node_end {
        let child = parse_node(bytes, child_start, node_end);
        child_start = align(child_start + usize::from(u16_at(bytes, child_start)));
        children.push(child);
    }
    Node {
        value_type,
        key,
        value,
        children,
    }
}

fn text_value(node: &Node) -> String {
    utf16_text(&node.value[..node.value.len() - 2])
}

#[test]
fn res_file_holds_an_empty_entry_then_the_version_resource() {
    let strings = native_oidc_strings("1.2.3");
    let res = encode_res([1, 2, 3, 0], &strings);
    let data = version_info([1, 2, 3, 0], &strings);
    assert_eq!(res.len() % 4, 0);

    // The empty leading entry: no data, a 32-byte header, type and name 0.
    assert_eq!(u32_at(&res, 0), 0);
    assert_eq!(u32_at(&res, 4), 32);
    assert_eq!(&res[8..16], &[0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0]);
    assert!(res[16..32].iter().all(|&byte| byte == 0));

    // The version resource header.
    let header = &res[32..64];
    assert_eq!(u32_at(header, 0) as usize, data.len());
    assert_eq!(u32_at(header, 4), 32);
    assert_eq!(u16_at(header, 8), 0xFFFF);
    assert_eq!(u16_at(header, 10), RT_VERSION);
    assert_eq!(u16_at(header, 12), 0xFFFF);
    assert_eq!(u16_at(header, 14), VERSION_RESOURCE_NAME);
    assert_eq!(u32_at(header, 16), 0);
    assert_eq!(u16_at(header, 20), VERSION_MEMORY_FLAGS);
    assert_eq!(u16_at(header, 22), LANGUAGE_EN_US);
    assert_eq!(u32_at(header, 24), 0);
    assert_eq!(u32_at(header, 28), 0);

    assert_eq!(&res[64..64 + data.len()], data.as_slice());
    assert_eq!(res.len(), align(64 + data.len()));
}

#[test]
fn version_info_tree_matches_the_crate_version_and_product_name() {
    let strings = native_oidc_strings("0.9.0-rc.1");
    let data = version_info([0, 9, 0, 0], &strings);
    let root = parse_node(&data, 0, data.len());
    assert_eq!(usize::from(u16_at(&data, 0)), data.len());

    assert_eq!(root.key, "VS_VERSION_INFO");
    assert_eq!(root.value_type, 0);
    assert_eq!(root.value.len(), 52);
    let fixed: Vec<u32> = (0..13)
        .map(|index| u32_at(&root.value, index * 4))
        .collect();
    assert_eq!(fixed[0], FIXED_FILE_INFO_SIGNATURE);
    assert_eq!(fixed[1], 0x0001_0000);
    // File and product version 0.9.0.0.
    assert_eq!(&fixed[2..6], &[0x0000_0009, 0, 0x0000_0009, 0]);
    assert_eq!(fixed[8], 0x0004_0004);
    assert_eq!(fixed[9], FILE_TYPE_DLL);

    let keys: Vec<&str> = root
        .children
        .iter()
        .map(|child| child.key.as_str())
        .collect();
    assert_eq!(keys, ["StringFileInfo", "VarFileInfo"]);

    let string_file_info = &root.children[0];
    assert_eq!(string_file_info.children.len(), 1);
    let table = &string_file_info.children[0];
    assert_eq!(
        table.key,
        format!("{LANGUAGE_EN_US:04X}{CODE_PAGE_UTF16:04X}")
    );
    assert_eq!(table.key, "040904B0");
    let parsed: Vec<(String, String)> = table
        .children
        .iter()
        .map(|string| {
            assert_eq!(string.value_type, 1);
            assert!(string.children.is_empty());
            (string.key.clone(), text_value(string))
        })
        .collect();
    let expected: Vec<(String, String)> = strings
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect();
    assert_eq!(parsed, expected);
    let field = |name: &str| {
        parsed
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(field("ProductName"), Some("NativeOIDC"));
    assert_eq!(field("ProductVersion"), Some("0.9.0-rc.1"));
    assert_eq!(field("FileVersion"), Some("0.9.0-rc.1"));
    assert_eq!(field("OriginalFilename"), Some("native_oidc.dll"));
    assert_eq!(field("CompanyName"), Some("ProjectFlora"));

    let var_file_info = &root.children[1];
    assert_eq!(var_file_info.children.len(), 1);
    let translation = &var_file_info.children[0];
    assert_eq!(translation.key, "Translation");
    assert_eq!(translation.value_type, 0);
    assert_eq!(
        translation.value,
        [LANGUAGE_EN_US.to_le_bytes(), CODE_PAGE_UTF16.to_le_bytes()].concat()
    );
}

#[test]
fn numeric_version_packs_each_part_into_sixteen_bits() {
    let data = version_info([65535, 1, 258, 7], &native_oidc_strings("65535.1.258"));
    let root = parse_node(&data, 0, data.len());
    assert_eq!(u32_at(&root.value, 8), 0xFFFF_0001);
    assert_eq!(u32_at(&root.value, 12), 0x0102_0007);
    assert_eq!(u32_at(&root.value, 16), 0xFFFF_0001);
    assert_eq!(u32_at(&root.value, 20), 0x0102_0007);
}
