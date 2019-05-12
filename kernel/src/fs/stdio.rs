//! Implement INode for Stdin & Stdout

use alloc::{collections::vec_deque::VecDeque, string::String, sync::Arc, vec::Vec};
use core::any::Any;
use spin::RwLock;

use rcore_fs::vfs;
use rcore_fs_sfs::*;
use rcore_fs::vfs::{INode, Metadata, FileSystem, FsError, FileType};

use crate::sync::Condvar;
use crate::sync::SpinNoIrqLock as Mutex;

use bcm2837::gpio;
use bcm2837::pwm_sound_device;

#[derive(Default)]
pub struct Stdin {
    buf: Mutex<VecDeque<char>>,
    pub pushed: Condvar,
}

impl Stdin {
    pub fn push(&self, c: char) {
        self.buf.lock().push_back(c);
        self.pushed.notify_one();
    }
    pub fn pop(&self) -> char {
        // QEMU v3.0 don't support M-mode external interrupt (bug?)
        // So we have to use polling.
        loop {
            let ret = self.buf.lock().pop_front();
            match ret {
                Some(c) => return c,
                None => self.pushed._wait(),
            }
        }
    }
    pub fn can_read(&self) -> bool {
        self.buf.lock().len() > 0
    }
}

#[derive(Default)]
pub struct Stdout;

#[derive(Default)]
pub struct Dsp {
    buf: Mutex<Vec<u8>>
}

#[derive(Default)]
pub struct GPIOOutput {
    pin: RwLock<u8>
}

impl GPIOOutput {
    fn new(init_pin: u8) -> Self {
        GPIOOutput {
            pin: RwLock::new(init_pin)
        }
    }
}

pub const STDIN_ID: usize = 0;
pub const STDOUT_ID: usize = 1;
pub const STDERR_ID: usize = 2;
pub const GPIO_ID: usize = 3;
pub const DSP_ID: usize = 4;

lazy_static! {
    pub static ref STDIN: Arc<Stdin> = Arc::new(Stdin::default());
    pub static ref STDOUT: Arc<Stdout> = Arc::new(Stdout::default());
    pub static ref GPIO: Arc<GPIOOutput> = Arc::new(GPIOOutput::new(0));
    pub static ref DSP: Arc<Dsp> = Arc::new(Dsp::default());
}

// TODO: better way to provide default impl?
macro_rules! impl_inode {
    () => {
        fn metadata(&self) -> vfs::Result<Metadata> { Err(FsError::NotSupported) }
        fn sync_all(&self) -> vfs::Result<()> { Ok(()) }
        fn sync_data(&self) -> vfs::Result<()> { Ok(()) }
        fn resize(&self, _len: usize) -> vfs::Result<()> { Err(FsError::NotSupported) }
        fn create(&self, _name: &str, _type_: FileType, _mode: u32) -> vfs::Result<Arc<INode>> { Err(FsError::NotDir) }
        fn unlink(&self, _name: &str) -> vfs::Result<()> { Err(FsError::NotDir) }
        fn link(&self, _name: &str, _other: &Arc<INode>) -> vfs::Result<()> { Err(FsError::NotDir) }
        fn move_(&self, _old_name: &str, _target: &Arc<INode>, _new_name: &str) -> vfs::Result<()> { Err(FsError::NotDir) }
        fn find(&self, _name: &str) -> vfs::Result<Arc<INode>> { Err(FsError::NotDir) }
        fn get_entry(&self, _id: usize) -> vfs::Result<String> { Err(FsError::NotDir) }
        fn fs(&self) -> Arc<FileSystem> { unimplemented!() }
        fn as_any_ref(&self) -> &Any { self }
        fn chmod(&self, _mode: u16) -> vfs::Result<()> { Ok(()) }
    };
}

impl INode for Stdin {
    fn read_at(&self, _offset: usize, buf: &mut [u8]) -> vfs::Result<usize> {
        buf[0] = self.pop() as u8;
        Ok(1)
    }
    fn write_at(&self, _offset: usize, _buf: &[u8]) -> vfs::Result<usize> {
        unimplemented!()
    }
    fn ioctl(&self, request: u32, data: *mut u8) -> Result<(), vfs::IOCTLError> { Ok(()) }
    impl_inode!();
}


impl INode for Stdout {
    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> vfs::Result<usize> {
        unimplemented!()
    }
    fn write_at(&self, _offset: usize, buf: &[u8]) -> vfs::Result<usize> {
        use core::str;
        //we do not care the utf-8 things, we just want to print it!
        let s = unsafe { str::from_utf8_unchecked(buf) };
        print!("{}", s);
        Ok(buf.len())
    }
    fn ioctl(&self, request: u32, data: *mut u8) -> Result<(), vfs::IOCTLError> { Ok(()) }
    impl_inode!();
}

impl INode for Dsp {
    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> vfs::Result<usize> {
        unimplemented!()
    }
    fn write_at(&self, _offset: usize, buf: &[u8]) -> vfs::Result<usize> {
        let tmp = &mut Vec::<u8>::from(buf);
        self.buf.lock().append(tmp);
        Ok(buf.len())
    }
    fn ioctl(&self, request: u32, data: *mut u8) -> Result<(), vfs::IOCTLError> {
        if request == 0 {
            // clear buffer and get ready for receiving audio data
            self.buf.lock().clear();
        } else if request == 1 {
            // play
            print!("dsp get {}", self.buf.lock().len());
            let mut sound_device = pwm_sound_device::PWMSoundDevice::new(44100, 2048);
            sound_device.init();
            let len = self.buf.lock().len() / 1;
            sound_device.Playback(self.buf.lock().as_ptr(), len, 1, 8);
            while sound_device.PlaybackActive() {
                // print!("waiting...");
                // do nothing
            }
            print!("play finish");
        }
        Ok(())
    }
    impl_inode!();
}


impl INode for GPIOOutput {
    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> vfs::Result<usize> {
        unimplemented!()
    }
    fn write_at(&self, _offset: usize, buf: &[u8]) -> vfs::Result<usize> {
        use core::str;
        let mut my_gpio = gpio::Gpio::<gpio::Uninitialized>::new(*self.pin.read()).into_output();
        my_gpio.set();
        Ok(0)
    }
    fn ioctl(&self, request: u32, data: *mut u8) -> Result<(), vfs::IOCTLError> {
        if (request > 53) {
            warn!("pin id > 53!");
            return Err(vfs::IOCTLError::NotValidParam);
        }
        let mut pin = self.pin.write();
        *pin = request as u8;
        Ok(())
    }
    impl_inode!();
}
