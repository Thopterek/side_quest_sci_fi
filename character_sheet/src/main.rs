use pdf_writer::types::{
    ActionType, AnnotationFlags, BorderType, FieldFlags, FieldType, FormActionFlags,
};
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
    fields.vartext_default_appearance(Str(b"/BasedFont 12 Tf 0 g"));

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
        .border_color_rgb(0.0, 0.0, 0.5);
    rectange_for_fields.flags(AnnotationFlags::PRINT);
    rectange_for_fields.finish();

    // group for toggling things if other are toggled
    let radio_group_id = Ref::new(4);
    let bounds_of_box = Rect::new(0.0, 0.0, 30.0, 18.0);
    let radio_buttons = [
        (
            Ref::new(20),
            Rect::new(108.0, 710.0, 138.0, 728.0),
            b"ChannelAlpha",
        ),
        (
            Ref::new(21),
            Rect::new(140.0, 710.0, 170.0, 728.0),
            b"Channel_Beta", // it's Like_That because it expects the same len for b"name"
        ),
        (
            Ref::new(22),
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
     * MK vs AP streams to be read about
     */
    let mut button_appearance = Content::new();
    button_appearance.save_state();
    button_appearance.set_stroke_rgb(0.0, 0.0, 0.5);
    button_appearance.rect(0.0, 0.0, 30.0, 18.0);
    button_appearance.stroke();
    button_appearance.begin_text();
    button_appearance.set_fill_gray(0.0);
    button_appearance.set_font(symbols_font_name, 14.0);
    button_appearance.show(Str(b"4"));
    button_appearance.end_text();
    button_appearance.restore_state();
    // Symbol font has to be added to stream for on
    let on_stream = button_appearance.finish();
    let mut on_appearance = pdf.form_xobject(radio_shown_id, &on_stream);
    on_appearance.bbox(bounds_of_box);
    on_appearance
        .resources()
        .fonts()
        .pair(symbols_font_name, symbols_font_id);
    on_appearance.finish();

    let mut off_appearance = Content::new();
    off_appearance.save_state();
    off_appearance.set_stroke_rgb(0.0, 0.0, 0.5);
    off_appearance.rect(0.0, 0.0, 30.0, 18.0);
    off_appearance.stroke();
    off_appearance.restore_state();
    let off_stream = off_appearance.finish();
    let mut off_xobject = pdf.form_xobject(radio_hidden_id, &off_stream);
    off_xobject.bbox(bounds_of_box);
    off_xobject.finish();

    /*
     * Creating widget annotations, setting up parent / child relation
     * then also mapping the sub dictionary to the rest of objects
     */
    for (id, rectangle, name) in radio_buttons {
        let mut field = pdf.form_field(id);
        field.parent(radio_group_id);
        let mut annotation = field.into_annotation();
        annotation.rect(rectangle).flags(AnnotationFlags::PRINT);
        annotation.appearance_state(Name(b"Off"));
        let mut full_appearance = annotation.appearance();
        full_appearance.normal().streams().pairs([
            (Name(name), radio_shown_id),
            (Name(b"Off"), radio_hidden_id),
        ]);
    }

    /*
     * Dropdown Menu with possible choices
     * first the standard setting of pieces to use
     * then showing the choices that can be made
     */
    let dropmenu_id = Ref::new(7);
    let mut dropmenu_field = pdf.form_field(dropmenu_id);
    dropmenu_field
        .partial_name(TextStr("choice"))
        .field_type(FieldType::Choice)
        .field_flags(FieldFlags::COMBO | FieldFlags::EDIT);
    dropmenu_field.choice_options().options([
        TextStr("Barbari"),
        TextStr("Auxilia"),
        TextStr("Obywatel"),
        TextStr("prefer not to say"),
    ]);
    let mut drop_annotation = dropmenu_field.into_annotation();
    drop_annotation
        .rect(Rect::new(108.0, 690.0, 208.0, 708.0))
        .flags(AnnotationFlags::PRINT);
    drop_annotation.finish();

    /*
     * PDF actions, activated on events
     * using push buttons -> they don't retain state
     * setting up to reset everything
     */
    let push_id = Ref::new(8);
    let mut push_field = pdf.form_field(push_id);
    push_field
        .partial_name(TextStr("Button"))
        .field_type(FieldType::Button)
        .field_flags(FieldFlags::PUSHBUTTON);
    let mut push_annotation = push_field.into_annotation();
    push_annotation
        .rect(Rect::new(108.0, 670.0, 138.0, 688.0))
        .flags(AnnotationFlags::PRINT);
    push_annotation
        .appearance_characteristics()
        .border_color_gray(0.5);
    push_annotation
        .action()
        .form_flags(FormActionFlags::INCLUDE_EXCLUDE)
        .action_type(ActionType::ResetForm)
        .fields();

    push_annotation.finish();

    /*
     * Sending the information to reader about
     * interactive forms that are part of the document
     */
    let catalog_id = Ref::new(9);
    let page_tree_id = Ref::new(10);
    let mut catalog_setup = pdf.catalog(catalog_id);
    catalog_setup.pages(page_tree_id).form().fields([
        text_field_id,
        radio_group_id,
        dropmenu_id,
        push_id,
    ]);
    catalog_setup.finish();

    /*
     * Creation of page and writing of sources
     */
    let page_id = Ref::new(11);
    let mut page = pdf.page(page_id);
    page.media_box(Rect::new(0.0, 0.0, 595.0, 842.0))
        .parent(page_tree_id)
        .resources()
        .fonts()
        .pair(base_font_name, base_font_id);
    page.annotations([
        text_field_id,
        radio_buttons[0].0,
        radio_buttons[1].0,
        radio_buttons[2].0,
        dropmenu_id,
        push_id,
    ]);
    page.finish();

    pdf.type1_font(base_font_id).base_font(Name(b"Helvetica"));
    pdf.type1_font(symbols_font_id)
        .base_font(Name(b"ZapfDingbats"));
    pdf.pages(page_tree_id).kids([page_id]).count(1);

    std::fs::write("target/forms.pdf", pdf.finish())
}
