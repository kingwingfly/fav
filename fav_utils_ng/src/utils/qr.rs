use crate::error::FavUtilsResult;
use qrcode::{render::unicode, QrCode};

pub fn show_qr_code(url: String) -> FavUtilsResult<()> {
    let code = QrCode::new(url).unwrap();
    let image = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build();
    println!("\n{}", image);
    Ok(())
}
