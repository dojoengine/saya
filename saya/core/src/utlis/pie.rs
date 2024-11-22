use std::env::temp_dir;

use cairo_vm::vm::runners::cairo_pie::CairoPie;

pub trait CairoPieBytes {
    fn to_bytes(&self) -> Vec<u8>;
}

impl CairoPieBytes for CairoPie {
    fn to_bytes(&self) -> Vec<u8> {
        let pie_dir = temp_dir().join("pie");
        self.write_zip_file(&pie_dir).unwrap();
        std::fs::read(pie_dir).unwrap()
    }
}
