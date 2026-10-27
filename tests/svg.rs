use ers::images::{SvgDecodeOptions, decode_svg};

#[path = "support/svg.rs"]
mod svg_support;
use svg_support::SvgBuilder;

#[test]
fn svg_shapes_render_into_native_pixels() {
    let image = decode_svg(&SvgBuilder::new().bytes(), &SvgDecodeOptions::default()).unwrap();
    assert_eq!(image.pixels.to_rgba8().get_pixel(4, 3).0, [255, 0, 0, 255]);
}

#[test]
fn svg_alpha_is_not_premultiplied_twice() {
    let mut builder = SvgBuilder::new();
    builder.body = "<rect width='100%' height='100%' fill='red' opacity='0.5'/>".into();
    let image = decode_svg(&builder.bytes(), &SvgDecodeOptions::default()).unwrap();
    assert_eq!(image.pixels.to_rgba8().get_pixel(4, 3).0, [255, 0, 0, 128]);
}

#[test]
fn svg_external_images_cannot_read_files_or_network_resources() {
    for href in [
        "file:///etc/passwd",
        "/etc/passwd",
        "https://example.org/private.png",
    ] {
        let mut builder = SvgBuilder::new();
        builder.body = format!("<image href='{href}' width='8' height='6'/>");
        let error = decode_svg(&builder.bytes(), &SvgDecodeOptions::default())
            .err()
            .unwrap();
        assert!(error.to_string().contains("External SVG"));
    }
}

#[test]
fn svg_document_types_are_rejected_before_entity_expansion() {
    let bytes = b"<!DOCTYPE svg [<!ENTITY secret SYSTEM 'file:///etc/passwd'>]><svg xmlns='http://www.w3.org/2000/svg' width='8' height='6'><text>&secret;</text></svg>";
    assert!(decode_svg(bytes, &SvgDecodeOptions::default()).is_err());
}

#[test]
fn svg_depth_limits_reject_nested_groups() {
    let mut builder = SvgBuilder::new();
    builder.body = format!(
        "{}<rect width='8' height='6'/>{}",
        "<g>".repeat(80),
        "</g>".repeat(80)
    );
    let error = decode_svg(&builder.bytes(), &SvgDecodeOptions::default())
        .err()
        .unwrap();
    assert!(error.to_string().contains("depth"));
}

#[test]
fn svg_reference_cycles_are_rejected_without_expansion() {
    let mut builder = SvgBuilder::new();
    builder.body = "<defs><g id='loop'><use href='#loop'/></g></defs><use href='#loop'/>".into();
    assert!(decode_svg(&builder.bytes(), &SvgDecodeOptions::default()).is_err());
}

#[test]
fn svg_canvas_limits_bound_the_prepared_pixels() {
    let mut builder = SvgBuilder::new();
    builder.width = 8192;
    builder.height = 8192;
    let image = decode_svg(&builder.bytes(), &SvgDecodeOptions::default()).unwrap();
    assert!(u64::from(image.pixels.width()) * u64::from(image.pixels.height()) <= 1024 * 1024);
    assert_eq!(image.pixels.to_rgba8().get_pixel(0, 0).0, [255, 0, 0, 255]);
}

#[test]
fn svg_mask_reference_chains_obey_the_depth_limit() {
    let mut builder = SvgBuilder::new();
    let masks = (0..80).map(|index| format!("<mask id='m{index}'><rect width='8' height='6' fill='white' mask='url(#m{})'/></mask>", index + 1)).collect::<String>();
    builder.body =
        format!("<defs>{masks}</defs><rect width='8' height='6' fill='red' mask='url(#m0)'/>");
    let error = decode_svg(&builder.bytes(), &SvgDecodeOptions::default())
        .expect_err("The mask reference chain bypassed the depth limit");
    assert!(error.to_string().contains("depth"));
}

#[test]
fn svg_stylesheet_references_obey_the_depth_limit() {
    let mut builder = SvgBuilder::new();
    let styles = (0..80)
        .map(|index| format!(".c{index} {{ mask: url('#m{}'); }}", index + 1))
        .collect::<String>();
    let masks = (0..80).map(|index| format!("<mask id='m{index}'><rect class='c{index}' width='8' height='6' fill='white'/></mask>")).collect::<String>();
    builder.body = format!(
        "<style>{styles}</style><defs>{masks}</defs><rect width='8' height='6' fill='red' mask='url(#m0)'/>"
    );
    let error = decode_svg(&builder.bytes(), &SvgDecodeOptions::default())
        .expect_err("The stylesheet bypassed SVG reference validation");
    assert!(error.to_string().contains("depth"));
}

#[test]
fn canceled_svg_requests_do_not_render() {
    let options = SvgDecodeOptions::default();
    options.cancellation.cancel();
    assert!(matches!(
        decode_svg(&SvgBuilder::new().bytes(), &options),
        Err(ers::Error::Canceled)
    ));
}

#[test]
fn svg_scripts_do_not_execute_or_replace_static_pixels() {
    let mut builder = SvgBuilder::new();
    builder.body = format!(
        "<script>throw new Error('must not execute');</script>{}",
        builder.body
    );
    let image = decode_svg(&builder.bytes(), &SvgDecodeOptions::default()).unwrap();
    assert_eq!(image.pixels.to_rgba8().get_pixel(4, 3).0, [255, 0, 0, 255]);
}
