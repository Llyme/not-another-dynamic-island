//! What the installer carries: the nadi.exe named by `NADI_EXE` (the installer script builds it first and names it), copied in;
//! and NADI's logo as the installer's own icon (written as a resource file the linker takes in).

use std::io::Write;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=NADI_EXE");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let exe = std::env::var("NADI_EXE").map(PathBuf::from).expect("NADI_EXE: the nadi.exe to carry (build it with installer.ps1)");
    println!("cargo:rerun-if-changed={}", exe.display());
    std::fs::copy(&exe, out.join("nadi.exe")).expect("NADI_EXE: no such file");

    let ico_path = "../nadi-shell/assets/nadi.ico";
    println!("cargo:rerun-if-changed={ico_path}");
    let ico = std::fs::read(ico_path).expect("nadi.ico");
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut res: Vec<u8> = Vec::new();
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
        group.extend_from_slice(&e[0..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut res, 14, 1, &group);
    // (Windows' newer controls: the dialog with a box to tick)
    let manifest = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3"><windowsSettings><dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness></windowsSettings></application>
</assembly>"#;
    entry(&mut res, 24, 1, manifest);
    let res_path = out.join("setup.res");
    std::fs::File::create(&res_path).unwrap().write_all(&res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", res_path.display());
}

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
