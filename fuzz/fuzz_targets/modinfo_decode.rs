#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(info) = lmod::modinfo::decode(data) {
        let _ = info.has_isr();
        let _ = lmod::modinfo::export_entries_offset(data);
        for index in 0..info.export_count.min(4) {
            let _ = lmod::modinfo::read_export(data, index);
        }
        for index in 0..4 {
            let _ = lmod::modinfo::read_res_meta(data, index);
        }
    }
});
