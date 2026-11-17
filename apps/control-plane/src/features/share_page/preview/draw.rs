//! 把 [`super::card`] 拼好的 SVG 画成 PNG：`resvg` 画字和圆，再把官网首页那三个机器人贴上去。
//!
//! 字体只有一份（思源黑体粗体，`apps/control-plane/assets/fonts/`），从文件读进来，一个进程只读一次；
//! 量字宽和画字用的是同一份字节。机器人是 `apps/web/site/robot-*.png`（从
//! `apps/web/public/studio/flat-robots.png` 抠出来给官网用的），每张四十来 KB，编进程序里。

use std::{path::Path, sync::Arc};

use resvg::{
    tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform},
    usvg::{self, ImageHrefResolver, fontdb},
};
use skrifa::{
    FontRef, MetadataProvider as _,
    charmap::Charmap,
    instance::{LocationRef, Size},
    metrics::GlyphMetrics,
};

use super::card::{self, HEIGHT, Measure, PreviewCard, ROBOTS, WIDTH};

/// 和 `card::ROBOTS` 同一个顺序：方的、圆的、胶囊的。
const ROBOT_IMAGES: [&[u8]; 3] = [
    include_bytes!("../../../../../web/site/robot-square.png"),
    include_bytes!("../../../../../web/site/robot-round.png"),
    include_bytes!("../../../../../web/site/robot-capsule.png"),
];

#[derive(Debug, thiserror::Error)]
pub(in crate::features::share_page) enum DrawError {
    #[error("读不了字体文件：{0}")]
    FontFile(#[from] std::io::Error),
    #[error("字体文件不是能用的字体")]
    FontData,
    #[error("机器人的图解不开")]
    Robot,
    #[error("拼出来的 SVG 读不了：{0}")]
    Svg(#[from] usvg::Error),
    #[error("画布建不起来")]
    Canvas,
    #[error("PNG 编不出来")]
    Encode,
}

/// 画图要用的：字体（量字宽一份，`resvg` 画字一份，是同一份字节）和三个机器人。
pub(in crate::features::share_page) struct PreviewAssets {
    font: Arc<Vec<u8>>,
    fonts: Arc<fontdb::Database>,
    family: String,
    robots: Vec<Pixmap>,
}

impl PreviewAssets {
    pub(in crate::features::share_page) fn load(path: &Path) -> Result<Self, DrawError> {
        let font = Arc::new(std::fs::read(path)?);
        FontRef::new(&font).map_err(|_| DrawError::FontData)?;
        let mut fonts = fontdb::Database::new();
        let ids = fonts.load_font_source(fontdb::Source::Binary(font.clone()));
        let family = ids
            .first()
            .and_then(|id| fonts.face(*id))
            .and_then(|face| face.families.first())
            .map(|(family, _)| family.clone())
            .ok_or(DrawError::FontData)?;
        fonts.set_sans_serif_family(family.clone());
        let robots = ROBOT_IMAGES
            .iter()
            .map(|image| Pixmap::decode_png(image).map_err(|_| DrawError::Robot))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            font,
            fonts: Arc::new(fonts),
            family,
            robots,
        })
    }

    /// 画一张，回 PNG 的字节。
    pub(in crate::features::share_page) fn draw(
        &self,
        card: &PreviewCard,
    ) -> Result<Vec<u8>, DrawError> {
        let svg = card::svg(card, &self.metrics()?);
        let options = usvg::Options {
            font_family: self.family.clone(),
            fontdb: self.fonts.clone(),
            // 图里不该有外面的图片：SVG 是我们自己拼的，用户写的字都转义了，这里再挡一道。
            image_href_resolver: ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(&svg, &options)?;
        let mut canvas = Pixmap::new(WIDTH, HEIGHT).ok_or(DrawError::Canvas)?;
        resvg::render(&tree, Transform::identity(), &mut canvas.as_mut());
        self.paste_robots(&mut canvas);
        canvas.encode_png().map_err(|_| DrawError::Encode)
    }

    /// 量字宽：和画字同一份字体。
    pub(in crate::features::share_page) fn metrics(&self) -> Result<FontMetrics<'_>, DrawError> {
        let font = FontRef::new(&self.font).map_err(|_| DrawError::FontData)?;
        Ok(FontMetrics {
            charmap: font.charmap(),
            glyphs: font.glyph_metrics(Size::new(1.0), LocationRef::default()),
        })
    }

    /// 机器人照 `object-fit: contain` 放进圆里那个正方形，居中。
    fn paste_robots(&self, canvas: &mut Pixmap) {
        let paint = PixmapPaint {
            quality: FilterQuality::Bicubic,
            ..PixmapPaint::default()
        };
        for (robot, spot) in self.robots.iter().zip(ROBOTS.iter()) {
            let (width, height) = (pixels(robot.width()), pixels(robot.height()));
            let scale = spot.image / width.max(height);
            let x = spot.center.0 - width * scale / 2.0;
            let y = spot.center.1 - height * scale / 2.0;
            canvas.draw_pixmap(
                0,
                0,
                robot.as_ref(),
                &paint,
                Transform::from_row(scale, 0.0, 0.0, scale, x, y),
                None,
            );
        }
    }
}

/// 机器人的图只有两三百像素，换成浮点不丢精度。
#[allow(clippy::cast_precision_loss)]
fn pixels(value: u32) -> f32 {
    value as f32
}

/// 按字体里的字宽量：一个 em 一个单位。
pub(in crate::features::share_page) struct FontMetrics<'a> {
    charmap: Charmap<'a>,
    glyphs: GlyphMetrics<'a>,
}

impl Measure for FontMetrics<'_> {
    fn advance(&self, character: char) -> Option<f32> {
        let glyph = self.charmap.map(character)?;
        self.glyphs.advance_width(glyph)
    }
}
