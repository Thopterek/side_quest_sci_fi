use pdf_writer::{Name, Pdf, Ref, TextStr, types::FieldType};

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
    Ok(())
}
