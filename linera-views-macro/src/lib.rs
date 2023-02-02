extern crate proc_macro;
extern crate syn;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, ItemStruct, __private::TokenStream2};

fn get_seq_parameter(generics: syn::Generics) -> Vec<syn::Ident> {
    let mut generic_vect = Vec::new();
    for param in generics.params {
        if let syn::GenericParam::Type(param) = param {
            generic_vect.push(param.ident);
        }
    }
    generic_vect
}

fn get_type_field(field: syn::Field) -> Option<syn::Ident> {
    match field.ty {
        syn::Type::Path(typepath) => {
            if let Some(x) = typepath.path.segments.into_iter().next() {
                return Some(x.ident);
            }
            None
        }
        _ => None,
    }
}

fn generate_view_code(input: ItemStruct) -> TokenStream2 {
    let struct_name = input.ident;
    let generics = input.generics;
    let template_vect = get_seq_parameter(generics.clone());
    let first_generic = template_vect
        .get(0)
        .expect("failed to find the first generic parameter");

    let mut names = Vec::new();
    let mut loades = Vec::new();
    let mut rollbackes = Vec::new();
    let mut flushes = Vec::new();
    let mut deletes = Vec::new();
    let mut cleares = Vec::new();
    for (idx, e) in input.fields.into_iter().enumerate() {
        let name = e.clone().ident.unwrap();
        let idx_lit = syn::LitInt::new(&idx.to_string(), Span::call_site());
        let type_ident = get_type_field(e).expect("Failed to find the type");
        loades.push(quote! {
            let index = #idx_lit;
            let base_key = context.derive_key(&index)?;
            let #name = #type_ident::load(context.clone_with_base_key(base_key)).await?;
        });
        names.push(quote! { #name });
        rollbackes.push(quote! { self.#name.rollback(); });
        flushes.push(quote! { self.#name.flush(batch)?; });
        deletes.push(quote! { self.#name.delete(batch); });
        cleares.push(quote! { self.#name.clear(); });
    }
    let first_name = names.get(0).expect("list of names should be non-empty");

    quote! {
        #[async_trait::async_trait]
        impl #generics linera_views::views::View<#first_generic> for #struct_name #generics
        where
            #first_generic: Context + Send + Sync + Clone + 'static,
            linera_views::views::ViewError: From<#first_generic::Error>,
        {
            fn context(&self) -> &#first_generic {
                self.#first_name.context()
            }

            async fn load(context: #first_generic) -> Result<Self, linera_views::views::ViewError> {
                #(#loades)*
                Ok(Self {#(#names),*})
            }

            fn rollback(&mut self) {
                #(#rollbackes)*
            }

            fn flush(&mut self, batch: &mut linera_views::common::Batch,) -> Result<(), linera_views::views::ViewError> {
                #(#flushes)*
                Ok(())
            }

            fn delete(self, batch: &mut linera_views::common::Batch,) {
                #(#deletes)*
            }

            fn clear(&mut self) {
                #(#cleares)*
            }
        }
    }
}

fn generate_save_delete_view_code(input: ItemStruct) -> TokenStream2 {
    let struct_name = input.ident;
    let generics = input.generics;
    let template_vect = get_seq_parameter(generics.clone());
    let first_generic = template_vect
        .get(0)
        .expect("failed to find the first generic parameter");

    let mut flushes = Vec::new();
    let mut deletes = Vec::new();
    for e in input.fields {
        let name = e.clone().ident.unwrap();
        flushes.push(quote! { self.#name.flush(&mut batch)?; });
        deletes.push(quote! { self.#name.delete(batch); });
    }

    quote! {
        #[async_trait::async_trait]
        impl #generics linera_views::views::ContainerView<#first_generic> for #struct_name #generics
        where
            #first_generic: Context + Send + Sync + Clone + 'static,
            linera_views::views::ViewError: From<#first_generic::Error>,
        {
            async fn save(&mut self) -> Result<(), linera_views::views::ViewError> {
                use linera_views::common::Batch;
                let mut batch = Batch::default();
                #(#flushes)*
                self.context().write_batch(batch).await?;
                Ok(())
            }

            async fn write_delete(self) -> Result<(), linera_views::views::ViewError> {
                use linera_views::common::Batch;
                let context = self.context().clone();
                let batch = Batch::build(move |batch| {
                    Box::pin(async move {
                        #(#deletes)*
                        Ok(())
                    })
                }).await?;
                context.write_batch(batch).await?;
                Ok(())
            }
        }
    }
}

fn generate_hash_view_code(input: ItemStruct) -> TokenStream2 {
    let struct_name = input.ident;
    let generics = input.generics;
    let template_vect = get_seq_parameter(generics.clone());
    let first_generic = template_vect
        .get(0)
        .expect("failed to find the first generic parameter");

    let field_hash = input
        .fields
        .into_iter()
        .map(|e| {
            let name = e.ident.unwrap();
            quote! { hasher.write_all(self.#name.hash().await?.as_ref())?; }
        })
        .collect::<Vec<_>>();

    quote! {
        #[async_trait::async_trait]
        impl #generics linera_views::views::HashableView<#first_generic> for #struct_name #generics
        where
            #first_generic: Context + Send + Sync + Clone + 'static,
            linera_views::views::ViewError: From<#first_generic::Error>,
        {
            type Hasher = linera_views::sha2::Sha512;

            async fn hash(&mut self) -> Result<<Self::Hasher as linera_views::views::Hasher>::Output, linera_views::views::ViewError> {
                use linera_views::views::{Hasher, HashableView};
                use std::io::Write;
                let mut hasher = Self::Hasher::default();
                #(#field_hash)*
                Ok(hasher.finalize())
            }
        }
    }
}

fn generate_crypto_hash_code(input: ItemStruct) -> TokenStream2 {
    let struct_name = input.ident;
    let generics = input.generics;
    let template_vect = get_seq_parameter(generics.clone());
    let first_generic = template_vect
        .get(0)
        .expect("failed to find the first generic parameter");

    let hash_type = syn::Ident::new(&format!("{}Hash", struct_name), Span::call_site());
    quote! {
        #[async_trait::async_trait]
        impl #generics linera_views::views::HashableContainerView<#first_generic> for #struct_name #generics
        where
            #first_generic: Context + Send + Sync + Clone + 'static,
            linera_views::views::ViewError: From<#first_generic::Error>,
        {
            async fn crypto_hash(&mut self) -> Result<linera_base::crypto::CryptoHash, linera_views::views::ViewError> {
                use linera_views::generic_array::GenericArray;
                use linera_views::common::Batch;
                use linera_base::crypto::{BcsHashable, CryptoHash};
                use linera_views::views::HashableView;
                use serde::{Serialize, Deserialize};
                use linera_views::sha2::{Sha512, Digest};
                #[derive(Serialize, Deserialize)]
                struct #hash_type(GenericArray<u8, <Sha512 as Digest>::OutputSize>);
                impl BcsHashable for #hash_type {}
                let hash = self.hash().await?;
                Ok(CryptoHash::new(&#hash_type(hash)))
            }
        }
    }
}

#[proc_macro_derive(View)]
pub fn derive_view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemStruct);
    generate_view_code(input).into()
}

#[proc_macro_derive(HashableView)]
pub fn derive_hash_view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemStruct);
    generate_hash_view_code(input).into()
}

#[proc_macro_derive(ContainerView)]
pub fn derive_container_view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemStruct);
    let mut stream = generate_view_code(input.clone());
    stream.extend(generate_save_delete_view_code(input));
    stream.into()
}

#[proc_macro_derive(HashableContainerView)]
pub fn derive_hash_container_view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemStruct);
    let mut stream = generate_view_code(input.clone());
    stream.extend(generate_save_delete_view_code(input.clone()));
    stream.extend(generate_hash_view_code(input.clone()));
    stream.extend(generate_crypto_hash_code(input));
    stream.into()
}

#[cfg(test)]
pub mod tests {

    use crate::*;
    use linera_views::{
        collection_view::CollectionView, common::Context, register_view::RegisterView,
    };
    use quote::quote;
    use syn::{parse_quote, token::Struct};

    #[test]
    #[rustfmt::skip]
    fn test_generate_view_code() {
        let input: ItemStruct = parse_quote!(
            struct TestView<C> {
                register: RegisterView<C, usize>,
                collection: CollectionView<C, usize, RegisterView<C, usize>>,
            }
        );
        let output = generate_view_code(input);

        let expected = quote!(
            #[async_trait::async_trait]
            impl<C> linera_views::views::View<C> for TestView<C>
            where
                C: Context + Send + Sync + Clone + 'static,
                linera_views::views::ViewError: From<C::Error>,
            {
                fn context(&self) -> &C {
                    self.register.context()
                }
                async fn load(context: C) -> Result<Self, linera_views::views::ViewError> {
                    let index = 0;
                    let base_key = context.derive_key(&index)?;
                    let register =
                        RegisterView::load(context.clone_with_base_key(base_key)).await?;
                    let index = 1;
                    let base_key = context.derive_key(&index)?;
                    let collection =
                        CollectionView::load(context.clone_with_base_key(base_key)).await?;
                    Ok(Self {
                        register,
                        collection
                    })
                }
                fn rollback(&mut self) {
                    self.register.rollback();
                    self.collection.rollback();
                }
                fn flush(
                    &mut self,
                    batch: &mut linera_views::common::Batch,
                ) -> Result<(), linera_views::views::ViewError> {
                    self.register.flush(batch)?;
                    self.collection.flush(batch)?;
                    Ok(())
                }
                fn delete(self, batch: &mut linera_views::common::Batch,) {
                    self.register.delete(batch);
                    self.collection.delete(batch);
                }
                fn clear(&mut self) {
                    self.register.clear();
                    self.collection.clear();
                }
            }
        );

        assert_eq!(output.to_string(), expected.to_string());
    }

    #[test]
    #[rustfmt::skip]
    fn test_generate_hash_view_code() {
        let input: ItemStruct = parse_quote!(
            struct TestView<C> {
                register: RegisterView<C, usize>,
                collection: CollectionView<C, usize, RegisterView<C, usize>>,
            }
        );
        let output = generate_hash_view_code(input);

        let expected = quote!(
            #[async_trait::async_trait]
            impl<C> linera_views::views::HashableView<C> for TestView<C>
            where
                C: Context + Send + Sync + Clone + 'static,
                linera_views::views::ViewError: From<C::Error>,
            {
                type Hasher = linera_views::sha2::Sha512;
                async fn hash(
                    &mut self
                ) -> Result<<Self::Hasher as linera_views::views::Hasher>::Output,
                    linera_views::views::ViewError
                > {
                    use linera_views::views::{Hasher, HashableView};
                    use std::io::Write;
                    let mut hasher = Self::Hasher::default();
                    hasher.write_all(self.register.hash().await?.as_ref())?;
                    hasher.write_all(self.collection.hash().await?.as_ref())?;
                    Ok(hasher.finalize())
                }
            }
        );

        assert_eq!(output.to_string(), expected.to_string());
    }

    #[test]
    fn test_generate_save_delete_view_code() {
        let input: ItemStruct = parse_quote!(
            struct TestView<C> {
                register: RegisterView<C, usize>,
                collection: CollectionView<C, usize, RegisterView<C, usize>>,
            }
        );
        let output = generate_save_delete_view_code(input);

        let expected = quote!(
            #[async_trait::async_trait]
            impl<C> linera_views::views::ContainerView<C> for TestView<C>
            where
                C: Context + Send + Sync + Clone + 'static,
                linera_views::views::ViewError: From<C::Error>,
            {
                async fn save(&mut self) -> Result<(), linera_views::views::ViewError> {
                    use linera_views::common::Batch;
                    let mut batch = Batch::default();
                    self.register.flush(&mut batch)?;
                    self.collection.flush(&mut batch)?;
                    self.context().write_batch(batch).await?;
                    Ok(())
                }
                async fn write_delete(self) -> Result<(), linera_views::views::ViewError> {
                    use linera_views::common::Batch;
                    let context = self.context().clone();
                    let batch = Batch::build(move |batch| {
                        Box::pin(async move {
                            self.register.delete(batch);
                            self.collection.delete(batch);
                            Ok(())
                        })
                    })
                    .await?;
                    context.write_batch(batch).await?;
                    Ok(())
                }
            }
        );

        assert_eq!(output.to_string(), expected.to_string());
    }
}
