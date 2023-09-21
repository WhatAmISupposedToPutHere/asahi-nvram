use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::unix::io::AsRawFd,
};

use crate::NvramWriter;

pub struct MtdWriter {
    file: File,
}

impl MtdWriter {
    pub fn new(file: File) -> MtdWriter {
        MtdWriter { file }
    }
}

impl NvramWriter for MtdWriter {
    fn erase_if_needed(&mut self, offset: u32, size: usize) {
        erase_if_needed(&self.file, offset, size);
    }

    fn write_all(&mut self, offset: u32, buf: &[u8]) -> std::io::Result<()> {
        self.file.seek(SeekFrom::Start(offset as u64))?;
        self.file.write_all(buf)?;

        Ok(())
    }
}

#[repr(C)]
pub struct EraseInfoUser {
    start: u32,
    length: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct MtdInfoUser {
    ty: u8,
    flags: u32,
    size: u32,
    erasesize: u32,
    writesize: u32,
    oobsize: u32,
    padding: u64,
}

nix::ioctl_write_ptr!(mtd_mem_erase, b'M', 2, EraseInfoUser);
nix::ioctl_read!(mtd_mem_get_info, b'M', 1, MtdInfoUser);

fn erase_if_needed(file: &File, offset: u32, size: usize) {
    if unsafe { mtd_mem_get_info(file.as_raw_fd(), &mut MtdInfoUser::default()) }.is_err() {
        return;
    }
    let erase_info = EraseInfoUser {
        start: offset,
        length: size as u32,
    };
    unsafe {
        mtd_mem_erase(file.as_raw_fd(), &erase_info).unwrap();
    }
}
