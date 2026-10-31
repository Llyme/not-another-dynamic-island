//! Puts NADI's logo into the exe as its icon (what Explorer shows): the .ico's pictures written as a Windows resource file (.res),
//! which the linker takes in. No resource compiler is needed.

use std::io::Write;

fn main() {
    println!("cargo:rerun-if-changed=assets/nadi.ico");
    let ico = std::fs::read("assets/nadi.ico").expect("assets/nadi.ico");
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut res: Vec<u8> = Vec::new();
    // (a .res file begins with an empty entry)
    res.extend_from_slice(&[0, 0, 0, 0, 0x20, 0, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0]);
    res.extend_from_slice(&[0; 16]);
    let mut group: Vec<u8> = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for i in 0..count {
        let e = &ico[6 + 16 * i..6 + 16 * (i + 1)];
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let at = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let id = (i + 1) as u16;
        entry(&mut res, 3, id, &ico[at..at + len]);
        // (the group's entry: the icon entry's first 12 bytes, then its id)
        group.extend_from_slice(&e[0..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut res, 14, 1, &group);
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("nadi.res");
    std::fs::File::create(&out).unwrap().write_all(&res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// One resource: its header (numbered type and name), then its data, kept to four-byte bounds.
fn entry(res: &mut Vec<u8>, kind: u16, name: u16, data: &[u8]) {
    res.extend_from_slice(&(data.len() as u32).to_le_bytes());
    res.extend_from_slice(&32u32.to_le_bytes());
    res.extend_from_slice(&[0xFF, 0xFF]);
    res.extend_from_slice(&kind.to_le_bytes());
    res.extend_from_slice(&[0xFF, 0xFF]);
    res.extend_from_slice(&name.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes());
    res.extend_from_slice(&0x1030u16.to_le_bytes());
    res.extend_from_slice(&0x0409u16.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes());
    res.extend_from_slice(data);
    while res.len() % 4 != 0 {
        res.push(0);
    }
}
