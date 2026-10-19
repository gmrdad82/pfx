use super::*;

impl Shell {
    pub fn bare(mut self) -> Shell {
        self.bare = true;
        self.strip = false;
        self.focus = Panel::C1;
        self
    }

    pub fn strip(mut self, on: bool) -> Shell {
        self.strip = on;
        self
    }

    pub fn is_bare(&self) -> bool {
        self.bare
    }

    pub fn set_clean(&mut self, on: bool) {
        self.clean = on;
    }

    pub fn is_clean(&self) -> bool {
        self.clean
    }

    pub(super) fn bare_view(&mut self, ui: &mut Ui, texture: &TextureHandle) {
        let palette = Palette::of(ui.ctx());
        CentralPanel::default()
            .frame(Frame::NONE.fill(palette.bg_deep))
            .show(ui, |ui| {
                let sense = if self.live() {
                    Sense::click_and_drag()
                } else {
                    Sense::hover()
                };
                let bar = if self.strip && self.span().is_some() {
                    transport::HEIGHT
                } else {
                    0.0
                };
                let room = (ui.available_size() - Vec2::new(0.0, bar)).max(Vec2::splat(1.0));
                let (area, response) = ui.allocate_exact_size(room, sense);
                let image = Rect::from_min_max(area.min.round(), area.max.round());
                self.viewport = Some(image);
                self.panels.push((Panel::C1, image));
                if !self.live() {
                    ui.painter().image(
                        texture.id(),
                        image,
                        viewport::cover(image, viewport::ASPECT),
                        Color32::WHITE,
                    );
                    return;
                }
                self.steer(ui, &response, image);
                self.picture(ui, image);
                if bar > 0.0 {
                    ui.painter().rect_filled(
                        Rect::from_min_max(
                            Pos2::new(image.left(), image.bottom()),
                            ui.max_rect().max,
                        ),
                        CornerRadius::ZERO,
                        palette.bg_base,
                    );
                    self.transport(ui);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_shell_gives_the_whole_portrait_frame_to_the_viewport() {
        let path = crate::scratch("bare-portrait");
        std::fs::write(
            &path,
            "recipe = 1\nname = \"bare\"\n\n[[object]]\nname = \"box\"\nkind = \"box\"\nsize = [0.2, 0.2, 0.2]\n",
        )
        .unwrap();
        let text = path.display().to_string();
        let ctx = Context::default();
        crate::theme::apply_theme(&ctx, &crate::theme::workbench());
        let script = pfx_editor_shell::Script::default();
        let mut bare = Shell::open(&text).without_recent().bare();
        pfx_editor_shell::drive(&ctx, [1080, 1920], &script, |ui| bare.ui(ui));
        assert_eq!(
            bare.viewport(),
            Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1080.0, 1920.0)))
        );
        assert_eq!(bare.focus(), Panel::C1);
        let mut panelled = Shell::open(&text).without_recent();
        pfx_editor_shell::drive(&ctx, [1080, 1920], &script, |ui| panelled.ui(ui));
        assert!(panelled.viewport().unwrap().width() < 540.0);
    }

    const BOX: &str = "recipe = 1\nname = \"lane\"\n\n[[object]]\nname = \"box\"\nkind = \"box\"\nsize = [2.0, 0.2, 2.0]\n";
    const ROUTE: &str = "\n[[route]]\nname = \"lane\"\npoints = [[-0.8, 0.1, -0.8], [0.0, 0.1, 0.6], [0.8, 0.1, -0.8]]\n";

    fn shapes(name: &str, text: &str, clean: bool) -> Vec<egui::epaint::ClippedShape> {
        let path = crate::scratch(name);
        std::fs::write(&path, text).unwrap();
        let mut viewer = crate::viewer::Viewer::new(&path, crate::follow::Mode::Fake);
        let mut shell = Shell::open(&path.display().to_string())
            .without_recent()
            .bare();
        viewer.sync(&mut shell);
        shell.routes.all = true;
        shell.set_clean(clean);
        let script = pfx_editor_shell::Script::default();
        let frame = |shell: &mut Shell| {
            let ctx = Context::default();
            crate::theme::apply_theme(&ctx, &crate::theme::workbench());
            pfx_editor_shell::drive(&ctx, [640, 480], &script, |ui| shell.ui(ui)).shapes
        };
        frame(&mut shell);
        viewer.sync(&mut shell);
        shell.view_error = None;
        frame(&mut shell)
    }

    #[test]
    fn a_clean_take_draws_no_overlay_and_a_hidden_route_draws_none() {
        let routed = format!("{BOX}{ROUTE}");
        let hidden = format!("{BOX}{ROUTE}show = false\n");
        let plain = shapes("clean-plain", BOX, false);
        assert_ne!(
            shapes("clean-routed", &routed, false),
            plain,
            "a shown route draws its line and handles"
        );
        assert_eq!(shapes("clean-hidden", &hidden, false), plain);
        let bare = shapes("clean-plain-clean", BOX, true);
        assert_ne!(bare, plain, "clean leaves the grid, axes and scale out");
        assert_eq!(shapes("clean-routed-clean", &routed, true), bare);
    }
}
