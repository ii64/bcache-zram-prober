use core::time;
use std::{fs::{read_dir, read, write}, path::{PathBuf, Path}, process::Command, io::{self, Write}};

fn find_devs<P>(prefix: P) -> Vec<PathBuf>
where
    P: AsRef<Path>
{
    read_dir("/dev").expect("failed to list dev ent")
        .into_iter()
        .filter(|ent| ent.is_ok())
        .map(|ent| ent.unwrap().path())
        .filter(|path| 
            path.to_str().unwrap()
                .starts_with(prefix.as_ref().to_str().unwrap()))
        .collect()
}

#[derive(Debug)]
struct ZramDeviceParam {
    dev_path: PathBuf,
    mem_limit: String,
    disk_size: String,
    comp_alg: String,
}

impl ZramDeviceParam {
    fn sysfs_mapper(self) -> Vec<(&'static str, String)> {
        vec![
            ("comp_algorithm", self.comp_alg),
            ("mem_limit", self.mem_limit),
            ("disksize", self.disk_size),
        ]
    }
}


fn add_zram() -> PathBuf
{
    let bytes = read("/sys/class/zram-control/hot_add")
        .expect("failed to add zram dev");
    let content = String::from_utf8_lossy(&bytes[..bytes.len()-1]);
    let devnr: u32 = content
        .parse()
        .expect("failed to parse zram nr");
    let devpath = Path::new("/dev")
        .join(format!("zram{}", devnr));
    devpath
}
fn setup_zram(opt: ZramDeviceParam) {
    let devname = opt.dev_path.file_name().unwrap();
    let blk = Path::new("/sys/block").join(devname);
    for (name, value) in opt.sysfs_mapper() {
        match write(blk.join(name), value) {
            Ok(_) => {},
            Err(e) => eprintln!("failed setup zram: {}: {}", name, e),
        }
    }
}

#[derive(Debug, Clone)]
struct MakeBcacheParam {
    cache_dev: String,
    backing_dev: String,

    bucket_size: String,
    block_size: String,
    cache_mode: String,
    sequential_cutoff: String,
}

impl MakeBcacheParam {
    fn sysfs_mapper(self) -> Vec<(&'static str, String)> {
        vec![
            ("sequential_cutoff", self.sequential_cutoff),
            ("cache_mode", self.cache_mode),
        ]
    }
}

impl Default for MakeBcacheParam {
    fn default() -> Self {
        Self {
            cache_dev: "".into(),
            backing_dev: "".into(),
            block_size: "4k".into(),
            bucket_size: "2M".into(),
            sequential_cutoff: "5M".into(),
            cache_mode: "writeback".into(),
        }
    }
}

fn waitk() {
    // adjust the wait kernel duration
    println!("wait for 2s...");
    std::thread::sleep(time::Duration::from_secs(2));
}

fn make_bcache(opt: MakeBcacheParam) {
    let output = Command::new("make-bcache")
        .arg("--wipe-bcache")
        .args(["--block", opt.block_size.as_str()])
        .args(["--bucket", opt.bucket_size.as_str()])
        .args(["-C", opt.cache_dev.as_str()])
        .args(["-B", opt.backing_dev.as_str()])
        
        .output()
        .expect("failed to exec make-bcache");
    io::stdout().write_all(&output.stdout).unwrap();
    io::stderr().write_all(&output.stderr).unwrap();
}

fn setup_bcache(bcache_device: PathBuf, opt: MakeBcacheParam) {
    let bcache_name = bcache_device.file_name().unwrap();
    let  bcache_dev_bcache_path = Path::new("/sys/block").to_path_buf()
        .join(bcache_name)
        .join("bcache");
    for (name, value) in opt.sysfs_mapper() {
        match write(bcache_dev_bcache_path.join(name), value) {
            Ok(_) => {},
            Err(e) => eprintln!("failed to set bcache: {}: {}", name, e),
        }
    }
}

fn main() {
    let zram_devices: Vec<_> = find_devs("/dev/zram");
    let bcache_devices: Vec<_> = find_devs("/dev/bcache").into_iter()
        .filter(|e| e.to_str().unwrap() != "/dev/bcache")
        .collect();
    
    let backing_device = Some(Path::new("/dev/sda7"));
    let cache_device = Some(Path::new("/dev/zram1"));
    // let bcache_device = Some(Path::new("/dev/bcache0"));

    println!("ZRAMDEV={:?} BCACHEDEV={:?}", zram_devices, bcache_devices);

    // check if backing device has been used for bcache
    if let Some(bdev) = backing_device {
        let bname = bdev.file_name().expect("backing dev not specified");
        let bname_str = bname.to_string_lossy();
        if bname_str.starts_with("sd") {
            let bdev_bcache_path = &Path::new("/sys/block").to_path_buf()
                .join(bname_str.get(0..3).expect("invalid sd device name"))
                .join(bname)
                .join("bcache");
            if read_dir(bdev_bcache_path).is_ok() {
                // block has active bcache
                let bdev_bcache_stop_path = bdev_bcache_path.join("stop");
                write(&bdev_bcache_stop_path, "1").expect("unable to stop bcache on backing device");
            }
        }
    }

    waitk();

    let cache_dev = if let Some(cdev) = cache_device {
            Path::new(cdev).to_path_buf()
        } else {
            add_zram()
    };
    let zram_opt = ZramDeviceParam {
        dev_path: cache_dev.clone(),
        mem_limit: "3G".into(), 
        disk_size: "3G".into(), 
        comp_alg: "zstd".into(),
    };
    setup_zram(zram_opt);


    let bcache_opt = MakeBcacheParam {
        backing_dev: backing_device.unwrap().to_path_buf().into_os_string().into_string().unwrap(),
        cache_dev: cache_dev.into_os_string().into_string().unwrap(),
        ..Default::default()
    };
    make_bcache(bcache_opt.clone());

    waitk();

    let bcache_devices_new: Vec<_> = find_devs("/dev/bcache").into_iter()
        .filter(|e| e.to_str().unwrap() != "/dev/bcache")
        .collect();
    let mut bcache_dev_diff: Vec<_> = bcache_devices_new
        .iter()
        .filter(|e| bcache_devices.contains(e))
        .collect();
    
    // get created bcache device
    println!("BCACHEDEV={:?}", bcache_devices_new);
    println!("BCACHEDEV_DIFF={:?}", bcache_dev_diff);
    let mut candidate: Option<&PathBuf> = None;
    if bcache_devices_new.len() == 1 {
        candidate = Some(bcache_devices_new.get(0).unwrap());
    } else if bcache_devices.len() == 1 {
        candidate = Some(bcache_devices.get(0).unwrap());
    }
    if bcache_dev_diff.len() > 1 || candidate.is_none() {
        panic!("unable to determine created bcache device")
    }
    bcache_dev_diff.push(candidate.unwrap());
    let bcache_device = bcache_dev_diff.get(0).unwrap().to_path_buf();

    setup_bcache(bcache_device, bcache_opt);
    
    if let Some(cdev) = cache_device {
        match write("/sys/fs/bcache/register", cdev.as_os_str().to_str().unwrap()) {
            Ok(_) => {},
            Err(e) => eprintln!("failed to register cache device to bcache: {}", e),
        }
    }
    
    // sudo mke2fs -t ext4 -O ^has_journal -F /dev/bcache0
    // sudo mkfs.btrfs /dev/bcache0
}
