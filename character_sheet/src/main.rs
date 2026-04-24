use pdf_writer::types::{AnnotationFlags, BorderType, FieldType};
use pdf_writer::{Finish, Name, Pdf, Rect, Ref, TextStr};

fn main() -> std::io::Result<()> {
    let mut pdf = Pdf::new();

    let base_font_id: Ref = Ref::new(1);
    let base_font_name: Name = Name(b"Base");

    let symbols_font_id: Ref = Ref::new(2);
    let symbols_font_name: Name = Name(b"Symbol");

    let text_field_id: Ref = Ref::new(3);
    let mut fields = pdf.form_field(text_field_id);
    fields.partial_name(TextStr("Fields"));
    fields.field_type(FieldType::Text);
    fields.text_value(TextStr(""));
    fields.text_default_value(TextStr(""));

    let mut rectange_for_fields = fields.into_annotation();
    rectange_for_fields.rect(Rect::new(108.0, 730.0, 208.0, 748.0));
    rectange_for_fields
        .border_style()
        .style(BorderType::Underline);
    rectange_for_fields
        .appearance_characteristics()
        .border_color_rgb(0.24, 0.24, 0.24);
    rectange_for_fields.flags(AnnotationFlags::PRINT);
    rectange_for_fields.finish();

    Ok(())
}
