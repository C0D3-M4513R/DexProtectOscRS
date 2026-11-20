pub struct Image{
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: &'static [u8],
}
impl From<Image> for egui::IconData {
    fn from(value: Image) -> Self {
        Self {
            width: value.width,
            height: value.height,
            rgba: Vec::from(value.rgba),
        }
    }
}
pub const ICON_BYTES:Image = ::app_macro::include_image!("../../images/app.png");