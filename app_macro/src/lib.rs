use proc_macro::Span;
use std::path::PathBuf;
use quote::quote;

#[proc_macro]
pub fn include_tree(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::LitStr);
    let span = Span::call_site();
    let source = span.local_file().expect("Unable to get the file, in which the macro was called from");
    let source = source.as_path();
    let source = source.parent().unwrap_or(source);
    let source = source.join(input.value());
    let mut out = proc_macro2::TokenStream::new();

    for i in std::fs::read_dir(source.as_path()).expect(format!("Unable to read dir: {}", source.display()).as_str()) {
        let i = i.expect(format!("Failed to read dir entry for dir: {}", source.display()).as_str());
        let path = PathBuf::from(i.file_name());
        if path.extension().map_or(true, |v|v.to_str().map_or(true, |v|v != "key")) {
            println!("Skipping non key-file: {}", path.display());
            continue;
        }

        let name = path.file_stem().expect(format!("Unable to get file name for {}", path.display()).as_str());
        let name = name.to_str().expect(format!("Unable to convert {} to string", name.display()).as_str());
        let path = source.join(&path);
        let buf = std::fs::read(&path).expect(format!("Unable to read file {} :", path.display()).as_str());

        let ts = quote::quote! {
            #name => &[#(#buf),*],
        };
        out.extend(ts);
    }

    quote! {::phf::phf_map! { #out } }.into()
}

#[proc_macro]
pub fn include_image(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::LitStr);
    let span = Span::call_site();
    let source = span.local_file().expect("Unable to get the file, in which the macro was called from");
    let source = source.as_path();
    let source = source.parent().unwrap_or(source);
    let source = source.join(input.value());
    let image = std::fs::read(&source).expect(format!("Unable to read file {} :", source.display()).as_str());
    let image = image::load_from_memory(image.as_slice()).expect("Unable to parse image");
    let image = image.into_rgba8();
    let width = image.width();
    let height = image.height();
    let rgba = image.into_raw();

    quote!{
        crate::Image {
            width: #width,
            height: #height,
            rgba: &[#(#rgba),*],
        }
    }.into()
}