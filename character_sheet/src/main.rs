use pdf_writer::types::{AnnotationFlags, BorderType, FieldFlags, FieldType};
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

fn main() -> std::io::Result<()> {
    let mut pdf = Pdf::new();

    /*
     * Bunch of references, each piece of PDF needs it
     * and the name to be used for particular ID
     */
    let base_font_id: Ref = Ref::new(1);
    let base_font_name: Name = Name(b"BasedFont");
    let symbols_font_id: Ref = Ref::new(2);
    let symbols_font_name: Name = Name(b"SymbolFont");
    let text_field_id: Ref = Ref::new(3);
    // Dictionary for the text field
    let mut fields = pdf.form_field(text_field_id);

    /*
     * Setting up what is being stored and where in text field
     * as per text_value -> stores the input of the user
     * text_default_value -> what happens after reset
     */
    fields.partial_name(TextStr("Fields"));
    fields.field_type(FieldType::Text);
    fields.text_value(TextStr("TextStorage"));
    fields.text_default_value(TextStr("ResetFormValue"));

    /*
     * I will have to do a recap on this one
     * Oh boy that's a huge one
     */
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

    // group for toggling things if other are toggled
    let radio_group_id = Ref::new(4);
    let bounds_of_box = Rect::new(0.0, 0.0, 30.0, 18.0);
    let radio_buttons = [
        (
            Ref::new(5),
            Rect::new(108.0, 710.0, 138.0, 728.0),
            b"ChannelAlpha",
        ),
        (
            Ref::new(6),
            Rect::new(140.0, 710.0, 170.0, 728.0),
            b"Channel_Beta", // it's Like_That because it expects the same len for b"name"
        ),
        (
            Ref::new(7),
            Rect::new(172.0, 710.0, 202.0, 728.0),
            b"ChannelGamma",
        ),
    ];

    /*
     * Creating a Radio parrent
     * all of the above buttons will be the children
     * most of the radio properties are defined through here
     */
    let mut button_field = pdf.form_field(radio_group_id);
    /*
     * Flags that are being used, plus a flag that doesn't work?
     * NO_TOGGLE_OFF -> once selected must be turned off with another one
     * btw | it's a bit OR operator on FieldFlags but closure on children method
     * _ is ignoring the value, while we take from tuples only id-s
     */
    button_field
        .partial_name(TextStr("Radio"))
        .field_type(FieldType::Button)
        .field_flags(
            FieldFlags::RADIO | FieldFlags::NO_TOGGLE_TO_OFF | FieldFlags::RADIOS_IN_UNISON,
        )
        .children(radio_buttons.map(|(id, _, _)| id));
    button_field.finish();

    let radio_shown_id = Ref::new(5);
    let radio_hidden_id = Ref::new(6);
    /*
     * Setting up how they will look
     * tick for shown, empty for hidden
     */
    let mut button_appearance = Content::new();
    button_appearance.save_state();
    button_appearance.begin_text();
    button_appearance.set_fill_gray(0.0);
    button_appearance.set_font(symbols_font_name, 14.0);
    button_appearance.show(Str(b"4"));
    button_appearance.end_text();
    button_appearance.restore_state();

    Ok(())
}
