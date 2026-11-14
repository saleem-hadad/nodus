//! Pictures on the board (SHAPE-006…SHAPE-008): picked from a file, pasted,
//! or dropped from Finder. Each is copied into the board's images folder
//! and put up as a picture shape at its own proportions.

use std::path::{Path, PathBuf};

use gpui::{Context, PathPromptOptions};
use nodus_board::geometry::Point;
use nodus_domain::{Rect, ShapeKind};

use super::canvas::BoardCanvas;
pub use nodus_board::pictures::{fitted, picture_size};

/// Pictures dropped together step this far apart.
const CASCADE: f64 = 32.0;

/// The kinds of picture the board takes, by extension.
pub fn is_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp"))
}

impl BoardCanvas {
    /// SHAPE-006: Add a picture…, put up in the middle of the view.
    pub(crate) fn pick_images(&mut self, cx: &mut Context<'_, Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let _ = this.update(cx, |c, cx| c.add_images(paths, None, cx));
        })
        .detach();
    }

    /// SHAPE-007, SHAPE-008: pictures from files, the first centred on
    /// `at` (or the view's middle), the rest stepping down from it.
    pub(crate) fn add_images(&mut self, paths: Vec<PathBuf>, at: Option<Point>, cx: &mut Context<'_, Self>) {
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| is_picture(p)).collect();
        if paths.is_empty() {
            return;
        }
        let nodus = self.state.read(cx).nodus.clone();
        let rt = super::core_rt(cx);
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            for (i, path) in paths.into_iter().enumerate() {
                let bytes = std::fs::read(&path).ok();
                let size = bytes.as_deref().and_then(picture_size);
                let (nodus, project_id) = (nodus.clone(), project_id.clone());
                let name =
                    super::on_core(&rt, async move { nodus.board().import_image(&project_id, &path).await }).await;
                let Ok(name) = name else { continue };
                let _ = this.update(cx, |c, cx| c.put_up_image(name, size, at, i, cx));
            }
        })
        .detach();
    }

    /// SHAPE-007: a picture pasted (its bytes and type).
    pub(crate) fn paste_image(&mut self, bytes: Vec<u8>, ext: &'static str, cx: &mut Context<'_, Self>) {
        let size = picture_size(&bytes);
        let nodus = self.state.read(cx).nodus.clone();
        let rt = super::core_rt(cx);
        // At the pointer when it's over the board, else the view's middle.
        let at = self
            .pointer
            .filter(|p| self.bounds.contains(p))
            .map(|p| self.to_flow(p));
        let project_id = self.project_id.clone();
        cx.spawn(async move |this, cx| {
            let name = super::on_core(
                &rt,
                async move { nodus.board().save_image(&project_id, bytes, ext).await },
            )
            .await;
            let Ok(name) = name else { return };
            let _ = this.update(cx, |c, cx| c.put_up_image(name, size, at, 0, cx));
        })
        .detach();
    }

    fn put_up_image(
        &mut self,
        name: String,
        size: Option<(u32, u32)>,
        at: Option<Point>,
        nth: usize,
        cx: &mut Context<'_, Self>,
    ) {
        let scale = nodus_board::ink_draw::size_at(self.viewport.zoom);
        let (w, h) = fitted(size, scale);
        let centre = at.unwrap_or_else(|| self.viewport_centre(nodus_board::geometry::Size { w: 0.0, h: 0.0 }));
        let step = CASCADE * scale * nth as f64;
        let rect = Rect {
            x: centre.x - w / 2.0 + step,
            y: centre.y - h / 2.0 + step,
            w,
            h,
        };
        self.add_shape(ShapeKind::Image, rect, String::new(), Some(name), false, cx);
    }

    /// Where a picture shape's file is; a server board's, once it's here
    /// (SRV-163).
    pub(crate) fn image_path(&self, name: &str, cx: &gpui::App) -> Option<PathBuf> {
        self.state.read(cx).nodus.board().image_path(&self.project_id, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_picture_files_by_their_extension() {
        assert!(is_picture(Path::new("/a/b.JPG")) && !is_picture(Path::new("/a/b.txt")));
    }
}
