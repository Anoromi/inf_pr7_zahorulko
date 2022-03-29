use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, FieldsNamed};

#[proc_macro_derive(VariableSaveD)]
pub fn variable_save(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, data, generics, .. } = parse_macro_input!(input);
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    match data {
        syn::Data::Struct(v) => {
            match v.fields {
                syn::Fields::Named(FieldsNamed { named, .. }) => {
                    let idents = named.iter().map(|f| &f.ident);
                    let idents2 = named.iter().map(|f| &f.ident);
                    let types = named.iter().map(|f| &f.ty);
                    let res = quote! {
                        #[async_trait]
                        impl #impl_generics VariableSave for #ident #ty_generics #where_clause {
                            async fn variable_save(&mut self, writer: &mut BufWriter<File>) -> Result<usize, Error> {
                                let mut accumulator : usize = 0;
                                #(accumulator += self.#idents.variable_save(writer).await?;) *
                                Ok(accumulator)
                            }
                            async fn variable_load(reader: &mut BufReader<File>) -> Result<Self, Error> {
                                Ok(Self {
                                    #(#idents2: #types::variable_load(reader).await?), *
                                })
                            }
                        }
                    };
                    res.into()
                }
                syn::Fields::Unnamed(_) => panic!("Not now"),
                syn::Fields::Unit => panic!("What is Unit?"),
            }
        }
        syn::Data::Enum(_) => panic!("Can't use on enums"),
        syn::Data::Union(_) => panic!("Can't use on unions"),
    }
}

