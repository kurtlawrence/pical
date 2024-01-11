use egui::{
    epaint::ImageDelta, ClippedPrimitive, Color32, Context, ImageData, Pos2, Rect, Rgba, Ui, Vec2,
};
use euc::{Buffer2d, Empty, Pipeline, Sampler, Texture};
use humantime::Duration;
use image::RgbaImage;
use std::{
    collections::HashMap,
    ops::{Add, Mul},
    time::Instant,
};

pub trait Render<C> {
    fn render(&self, ui: &mut Ui, ctx: C);
}

pub struct Painted {
    pub img: RgbaImage,
    pub ui_gen: Duration,
    pub tessellation: Duration,
    pub rendering: Duration,
    pub resizing: Option<Duration>,
}

impl Painted {
    pub fn log_debug_timings(&self) {
        let Self {
            img: _,
            ui_gen,
            tessellation,
            rendering,
            resizing,
        } = self;
        log::debug!("⏱ UI Generation: {ui_gen}");
        log::debug!("⏱ Tessallation: {tessellation}");
        log::debug!("⏱ Rendering: {rendering}");
        if let Some(x) = resizing {
            log::debug!("⏱ Resizing: {x}");
        }
    }
}

pub fn paint<F>(width_px: u32, height_px: u32, scaling: f32, run_ui: F) -> Painted
where
    F: FnOnce(&Context),
{
    // define the draw pixels
    let [width, height] = [width_px, height_px].map(|x| (x as f32 * scaling).floor() as u32);
    // define screen size in _points_
    let size = [width_px, height_px].map(|x| x as f32);

    // generate UI
    let now = Instant::now();
    let ctx = Context::default();
    let input = egui::RawInput {
        screen_rect: Rect::from_two_pos(Pos2::ZERO, size.into()).into(),
        ..Default::default()
    };
    let output = ctx.run(input.clone(), run_ui);
    let ui_gen = Duration::from(now.elapsed());

    // generate painting triangles
    let now = Instant::now();
    let meshes = ctx
        .tessellate(output.shapes, output.pixels_per_point)
        .into_iter()
        .filter_map(|x| Mesh::from_clipped_prim(size, x))
        .collect::<Vec<_>>();
    let tessellation = Duration::from(now.elapsed());

    // populate the textures
    let now = Instant::now();
    let txs: HashMap<_, _> = output
        .textures_delta
        .set
        .into_iter()
        .map(|(id, delta)| (id, RgbaTexture::from(delta)))
        .collect();

    let mut colour_buf = Buffer2d::fill(
        [width as usize, height as usize],
        Rgba::from_black_alpha(0.),
    );

    for mut mesh in meshes {
        let sampler = txs.get(&mesh.mesh.texture_id).map(|tx| tx.linear());
        mesh.sampler = sampler;
        mesh.render(
            mesh.mesh
                .indices
                .iter()
                .copied()
                .map(|x| mesh.mesh.vertices[x as usize]),
            &mut colour_buf,
            &mut Empty::default(),
        );
    }

    // fill image
    let i = buf_to_img(width, height, &colour_buf);
    let rendering = Duration::from(now.elapsed());
    let (img, resizing) = if scaling == 1.0 {
        (i, None)
    } else {
        let now = Instant::now();
        let i = image::imageops::resize(
            &i,
            width_px,
            height_px,
            image::imageops::FilterType::Lanczos3,
        );
        (i, Some(Duration::from(now.elapsed())))
    };

    Painted {
        img,
        ui_gen,
        tessellation,
        rendering,
        resizing,
    }
}

fn buf_to_img(width: u32, height: u32, buf: &Buffer2d<Rgba>) -> RgbaImage {
    let mut img = RgbaImage::new(width, height);
    let pxs = buf.raw();

    for x in 0..width {
        for y in 0..height {
            let px = pxs[buf.linear_index([x as usize, y as usize])];
            img.put_pixel(x, y, Color32::from(px).to_array().into());
        }
    }

    img
}

struct Mesh<'a> {
    mesh: egui::Mesh,
    sampler: Option<euc::Linear<&'a RgbaTexture>>,
    half_size: Vec2,
}

impl<'a> Mesh<'a> {
    fn from_clipped_prim(size: [f32; 2], prim: ClippedPrimitive) -> Option<Self> {
        let ClippedPrimitive {
            clip_rect: _,
            primitive,
        } = prim;
        let half_size = Vec2::from(size) * 0.5;
        match primitive {
            egui::epaint::Primitive::Mesh(mesh) => Some(Mesh {
                mesh,
                sampler: None,
                half_size,
            }),
            egui::epaint::Primitive::Callback(_) => {
                log::warn!("custom primitive callback invoked");
                None
            }
        }
    }
}

impl<'a> Pipeline<'_> for Mesh<'a> {
    type Vertex = egui::epaint::Vertex;
    type VertexData = PipelineVertex;
    type Fragment = Rgba;
    type Primitives = euc::TriangleList;
    type Pixel = Rgba;

    fn vertex(&self, vertex: &Self::Vertex) -> ([f32; 4], Self::VertexData) {
        let egui::epaint::Vertex { pos, color, uv } = *vertex;
        let Vec2 { x, y } = pos.to_vec2() / self.half_size - Vec2::splat(1.0);
        let vd = PipelineVertex {
            colour: Rgba::from(color),
            uv: uv.to_vec2(),
        };
        ([x, y, 0.0, 1.0], vd)
    }

    fn fragment(&self, vs_out: Self::VertexData) -> Self::Fragment {
        let PipelineVertex { colour, uv } = vs_out;
        self.sampler
            .as_ref()
            .map(|sampler| sampler.sample(uv.into()) * colour)
            .unwrap_or(colour)
    }

    fn blend(&self, old: Self::Pixel, new: Self::Fragment) -> Self::Pixel {
        // all old, new, and output are premultiplied
        new + old.multiply(1.0 - new.a())
    }

    fn rasterizer_config(
            &self,
    ) -> <<Self::Primitives as euc::primitives::PrimitiveKind<Self::VertexData>>::Rasterizer as euc::rasterizer::Rasterizer>::Config{
        euc::CullMode::None
    }

    fn coordinate_mode(&self) -> euc::CoordinateMode {
        euc::CoordinateMode::OPENGL.without_z_clip()
    }
}

#[derive(Copy, Clone)]
struct PipelineVertex {
    colour: Rgba,
    uv: Vec2,
}

impl Mul<f32> for PipelineVertex {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self::Output {
        Self {
            colour: self.colour * rhs,
            uv: self.uv * rhs,
        }
    }
}

impl Add for PipelineVertex {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            colour: self.colour + rhs.colour,
            uv: self.uv + rhs.uv,
        }
    }
}

struct RgbaTexture {
    size: [usize; 2],
    pxs: Vec<Rgba>,
}

impl From<ImageDelta> for RgbaTexture {
    fn from(delta: ImageDelta) -> Self {
        assert!(delta.is_whole(), "assuming setting total texture each time");
        let size = delta.image.size();
        match delta.image {
            ImageData::Color(_) => todo!(),
            ImageData::Font(font) => RgbaTexture {
                size,
                pxs: font.srgba_pixels(None).map(Into::into).collect(),
            },
        }
    }
}

impl Texture<2> for RgbaTexture {
    type Index = usize;
    type Texel = Rgba;

    fn size(&self) -> [Self::Index; 2] {
        self.size
    }

    fn read(&self, index: [Self::Index; 2]) -> Self::Texel {
        let [x, y] = index;
        let i = y * self.size[0] + x;
        self.pxs[i]
    }
}
