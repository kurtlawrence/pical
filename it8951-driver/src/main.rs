use clap::Parser;
use image::GrayImage;
use it8951::WaveformMode;
use miette::*;
use std::path::Path;

fn main() -> Result<()> {
    let app = App::parse();

    let driver = app.build_driver()?;
    if app.test {
        run_test(driver)
    } else {
        run(driver)
    }
}

/// Driver to display an image on `IT8951` devices, such as
/// https://core-electronics.com.au/waveshare-10-3inch-e-paper-display-hat-for-raspberry-pi-black-white.html
#[derive(Parser)]
struct App {
    /// The SPI device path.
    #[arg(long, default_value = "/dev/spidev0.0")]
    spi: String,

    /// The GPIO device path.
    #[arg(long, default_value = "/dev/gpiochip0")]
    gpio: String,

    /// Run a test routine for checking display is working correctly.
    #[arg(long)]
    test: bool,
}

impl App {
    fn build_driver(&self) -> Result<DriverRun> {
        use linux_embedded_hal::{gpio_cdev::*, spidev::*, CdevPin, Delay, Spidev};
        let devspi = &self.spi;
        println!("ℹ Connecting to {devspi}");
        let mut spi = Spidev::open(devspi)
            .into_diagnostic()
            .wrap_err_with(|| format!("spi path: {devspi}"))?;
        let opts = SpidevOptions::new()
            .bits_per_word(8)
            .max_speed_hz(12_000_000)
            .mode(SpiModeFlags::SPI_MODE_0)
            .build();
        spi.configure(&opts).into_diagnostic()?;

        let devgpio = &self.gpio;
        let mut chip = Chip::new(devgpio)
            .into_diagnostic()
            .wrap_err_with(|| format!("gpio path: {devgpio}"))?;
        // RST: 17
        let rst_output = chip.get_line(17).into_diagnostic()?;
        let rst_output_handle = rst_output
            .request(LineRequestFlags::OUTPUT, 0, "meeting-room")
            .into_diagnostic()?;
        let rst = CdevPin::new(rst_output_handle).into_diagnostic()?;
        // BUSY / HDRY: 24
        let busy_input = chip.get_line(24).into_diagnostic()?;
        let busy_input_handle = busy_input
            .request(LineRequestFlags::INPUT, 0, "meeting-room")
            .into_diagnostic()?;
        let busy = CdevPin::new(busy_input_handle).into_diagnostic()?;

        let driver = it8951::interface::IT8951SPIInterface::new(spi, busy, rst, Delay);
        /* Disabled no reset for now
        let x = if self.reset {
            it8951::IT8951::new(driver).init(1670)
        } else {
            it8951::IT8951::attach(driver)
        }
        */
        let x = it8951::IT8951::new(driver)
            .init(1670)
            .map_err(|e| miette!("failed to build it8951 driver: {:?}", e))?;
        println!("✅ Connected to E-Ink Display:\n{:#?}", x.get_dev_info());
        Ok(Driver { inner: x })
    }
}

fn run_test(mut driver: DriverRun) -> Result<()> {
    let img = test_image();
    driver.push_image(&img, None, WaveformMode::GrayscaleClearing16)?;
    println!("✅ Display refreshed, you should see your image now!");
    driver.shutdown()
}

fn run(driver: DriverRun) -> Result<()> {
    let stdin = std::io::stdin();
    let mut line = String::new();
    let mut driver = driver.sleep()?;

    loop {
        line.clear();
        println!(
            "🔤 Please specifiy <IMAGE> [--high|--low|--reset] [<DIFF IMAGE>] path(s) to render"
        );
        stdin.read_line(&mut line).into_diagnostic()?;
        let (img, quality, diff) = parse_line(line.trim())?;
        let img = read_image(img)?;
        let mut diff = diff.map(read_image).transpose()?;
        let mut d = driver.wake()?;
        let mode = match quality {
            Quality::Reset => {
                diff = None;
                d.reset()?;
                WaveformMode::GrayscaleClearing16
            }
            Quality::High => WaveformMode::GrayscaleClearing16,
            Quality::Low => WaveformMode::DU4,
        };
        d.push_image(&img, diff.as_ref(), mode)?;
        driver = d.sleep()?;
        println!("✅ Display refreshed, you should see your image now!");
    }
}

enum Quality {
    Reset,
    High,
    Low,
}

fn parse_line(line: &str) -> Result<(&Path, Quality, Option<&Path>)> {
    let mut split = line.split_whitespace();
    let img = split
        .next()
        .map(Path::new)
        .ok_or_else(|| miette!("no image path given"))?;
    let mut quality = Quality::High;
    let mut diff = split.next();
    if diff.as_deref() == Some("--reset") {
        quality = Quality::Reset;
        diff = split.next();
    } else if diff.as_deref() == Some("--high") {
        quality = Quality::High;
        diff = split.next();
    } else if diff.as_deref() == Some("--low") {
        quality = Quality::Low;
        diff = split.next();
    }

    Ok((img, quality, diff.map(Path::new)))
}

struct Driver<State> {
    inner: it8951::IT8951<
        it8951::interface::IT8951SPIInterface<
            linux_embedded_hal::Spidev,
            linux_embedded_hal::CdevPin,
            linux_embedded_hal::CdevPin,
            linux_embedded_hal::Delay,
        >,
        State,
    >,
}

type DriverRun = Driver<it8951::Run>;

impl Driver<it8951::Run> {
    fn push_image(
        &mut self,
        img: &GrayImage,
        diff: Option<&GrayImage>,
        mode: WaveformMode,
    ) -> Result<()> {
        use it8951::memory_converter_settings::*;
        let it8951::DevInfo {
            panel_width,
            panel_height,
            memory_address,
            ..
        } = self.inner.get_dev_info();
        let cnvtr = || MemoryConverterSetting {
            endianness: MemoryConverterEndianness::LittleEndian,
            bit_per_pixel: MemoryConverterBitPerPixel::BitsPerPixel4,
            rotation: MemoryConverterRotation::Rotate0,
        };

        println!(
            "ℹ Pushing {}x{} image to display buffer",
            img.width(),
            img.height()
        );

        for (i, row) in enumerate_different_rows(img, diff).take(panel_height as usize) {
            let area = it8951::AreaImgInfo {
                area_x: 0,
                area_y: i as u16, // row index
                area_w: panel_width,
                area_h: 1,
            };
            let data =
                luma8_pxs_into_packed_u16_vec(row.take(panel_width as usize).map(|(_, _, px)| *px));
            self.inner
                .load_image_area(memory_address, cnvtr(), &area, &data)
                .map_err(|e| miette!("failed to write image row to memory: {:?}", e))?;
        }

        println!("✅ Buffer updated!");

        self.inner
            .display(mode)
            .map_err(|e| miette!("failed to display image buffer: {:?}", e))
    }

    fn reset(&mut self) -> Result<()> {
        self.inner
            .reset()
            .map_err(|e| miette!("failed to reset screen: {:?}", e))
    }

    fn sleep(self) -> Result<Driver<it8951::PowerDown>> {
        self.inner
            .sleep()
            .map_err(|e| miette!("failed to sleep device: {:?}", e))
            .map(|inner| Driver { inner })
    }

    fn shutdown(self) -> Result<()> {
        self.inner
            .sleep()
            .map_err(|e| miette!("failed to sleep device: {:?}", e))
            .map(|_| ())
    }
}

impl Driver<it8951::PowerDown> {
    fn wake(self) -> Result<Driver<it8951::Run>> {
        self.inner
            .sys_run()
            .map_err(|e| miette!("failed to wake device: {:?}", e))
            .map(|inner| Driver { inner })
    }
}

fn enumerate_different_rows<'a>(
    img: &'a GrayImage,
    diff: Option<&'a GrayImage>,
) -> impl Iterator<Item = (u32, image::buffer::EnumeratePixels<'a, image::Luma<u8>>)> {
    let mut diff = diff.into_iter().flat_map(|x| x.rows());
    img.enumerate_rows()
        .filter(move |(_, r)| match diff.next() {
            Some(d) => !r.clone().map(|(_, _, p)| p).eq(d),
            None => true,
        })
}

fn test_image() -> GrayImage {
    image::load_from_memory(include_bytes!("../test.png"))
        .expect("valid PNG file")
        .into_luma8()
}

fn read_image(file: impl AsRef<Path>) -> Result<GrayImage> {
    let file = file.as_ref();
    image::open(file)
        .into_diagnostic()
        .wrap_err_with(|| miette!("image path: {}", file.display()))
        .map(|x| x.into_luma8())
}

fn luma8_pxs_into_packed_u16_vec(pxs: impl Iterator<Item = image::Luma<u8>>) -> Vec<u16> {
    let mut pxs = pxs.collect::<Vec<_>>();
    pxs.reverse();
    pxs.chunks(4)
        .map(|run| {
            run.iter()
                .rev()
                .map(|x| x.0[0] / 16)
                .fold(0u16, |d, x| d << 4 | x as u16)
        })
        .collect()
}
