use clap::Parser;
use image::GrayImage;
use miette::*;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let app = App::parse();

    let img = if app.test {
        test_image()
    } else {
        app.load_image()?
    };
    // disable diff functionality for now
    let diff = app
        .diff
        .as_ref()
        .filter(|_| false)
        .map(read_image)
        .transpose()?;

    let mut driver = app.build_driver()?;
    driver.push_image(&img, diff.as_ref())?;
    println!("✅ Display refreshed, you should see your image now!");
    driver.shutdown()
}

/// Driver to display an image on `IT8951` devices, such as
/// https://core-electronics.com.au/waveshare-10-3inch-e-paper-display-hat-for-raspberry-pi-black-white.html
#[derive(Parser)]
struct App {
    /// The image to display.
    img: Option<PathBuf>,

    /// The SPI device path.
    #[arg(long, default_value = "/dev/spidev0.0")]
    spi: String,

    /// The GPIO device path.
    #[arg(long, default_value = "/dev/gpiochip0")]
    gpio: String,

    /// Reset device upon conncetion
    #[arg(long, short)]
    reset: bool,

    /// Only redraw **rows** of an image which differs this image.
    /// So `img` is the new image, `diff` is the old image.
    /// Implies `reset=false`.
    #[arg(long)]
    diff: Option<PathBuf>,

    /// Run a test routine for checking display is working correctly.
    #[arg(long)]
    test: bool,
}

impl App {
    fn load_image(&self) -> Result<GrayImage> {
        self.img
            .as_ref()
            .ok_or_else(|| miette!("please specify an image path"))
            .and_then(read_image)
    }

    fn build_driver(&self) -> Result<Driver> {
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

struct Driver {
    inner: it8951::IT8951<
        it8951::interface::IT8951SPIInterface<
            linux_embedded_hal::Spidev,
            linux_embedded_hal::CdevPin,
            linux_embedded_hal::CdevPin,
            linux_embedded_hal::Delay,
        >,
        it8951::Run,
    >,
}

impl Driver {
    fn push_image(&mut self, img: &GrayImage, diff: Option<&GrayImage>) -> Result<()> {
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
            .display(it8951::WaveformMode::GL16)
            .map_err(|e| miette!("failed to display image buffer: {:?}", e))
    }

    fn shutdown(self) -> Result<()> {
        self.inner
            .sleep()
            .map_err(|e| miette!("failed to sleep device: {:?}", e))
            .map(|_| ())
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
