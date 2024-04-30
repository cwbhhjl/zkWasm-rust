use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Field, Ident, Variant, Type};

struct Fd {
    name: Ident,
}

struct StructContext {
    name: Ident,
    fields: Vec<Fd>,
}

struct Ed {
    name: Ident,
    ty: Type,
}

struct EnumContext {
    name: Ident,
    variants: Vec<Ed>,
}

enum Context {
    S(StructContext),
    E(EnumContext),
}


impl From<Field> for Fd {
    fn from(f: Field) -> Self {
        Self {
            name: f.ident.unwrap()
        }
    }
}

use syn::Type::*;

fn debug_type<'a>(t: &Type) -> &'a str {
    match t {
        Array(_) => "Array",
        BareFn(_) => "BareFn",
        Group(_) => "Group",
        ImplTrait(_) => "ImplTrait",
        Infer(_) => "Infer",
        Macro(_) => "Macro",
        Never(_) => "Never",
        Paren(_) => "Paren",
        Path(_) => "Path",
        Ptr(_) => "Ptr",
        Reference(_) => "Reference",
        Slice(_) => "Slice",
        TraitObject(_) => "TraitObject",
        Tuple(_) => "Tuple",
        Verbatim(_) => "Verbatim",
        _ => todo!()
        // Not public API.
    }
}

fn get_ident (t: &Type) -> Ident {
    match t {
        Path(p) => p.path.get_ident().unwrap().clone(),
        _ => todo!("not implemented")
        // Not public API.
    }
}


impl From<Variant> for Ed {
    fn from(f: Variant) -> Self {
        let fields = f.fields.iter().collect::<Vec<_>>().clone();
        let t = fields[0].clone().ty;
        println!("tuple type is {}, fields number {}", debug_type(&t), fields.len());
        Self {
            name: f.ident,
            ty: t,
        }
    }
}

impl From<DeriveInput> for Context {
    fn from(input: DeriveInput) -> Self {
        let name = input.ident;
        match input.data {
            Data::Struct(r) => {
                let fds = r.fields.into_iter().map(Fd::from).collect();
                Self::S (StructContext { name, fields: fds })
            }
            Data::Enum(r) => {
                let variants = r.variants.into_iter().map(Ed::from).collect();
                Self::E (EnumContext { name, variants})
            }
            _ => {
                panic!("Unsupported data type")
            }
        }
    }
}

impl StructContext {
    pub fn witness_obj_render(&self) -> TokenStream2 {
        let name = self.name.clone();
        let fields_writer = self.witness_writer();
        let fields_reader = self.witness_reader();
        quote!(
            impl WitnessObjWriter for #name {
                fn to_witness(&self, ori_base: *const u8) {
                    #(#fields_writer)*
                }
            }

            impl WitnessObjReader for #name {
                fn from_witness(&mut self, fetcher: &mut impl FnMut() -> u64,  base: *const u8) {
                    unsafe {
                        #(#fields_reader)*
                    }
                }
            }
        )
    }

    fn witness_reader(&self) -> Vec<TokenStream2> {
        let mut ret = vec![];
        for i in 0..self.fields.len() {
            let name = self.fields[i].name.clone();
            ret.push(quote!(self.#name.from_witness(fetcher, base);));
        }
        ret
    }

    fn witness_writer(&self) -> Vec<TokenStream2> {
        let mut ret = vec![];
        for i in 0..self.fields.len() {
            let name = self.fields[i].name.clone();
            ret.push(quote!(self.#name.to_witness(ori_base);));
        }
        ret
    }
}


impl EnumContext {
    pub fn witness_obj_render(&self) -> TokenStream2 {
        let name = self.name.clone();
        let fields_writer = self.witness_writer();
        let fields_reader = self.witness_reader();
        quote!(
            impl WitnessObjWriter for #name {
                fn to_witness(&self, ori_base: *const u8) {
                    let obj = self as *const Self;
                    unsafe {
                        super::super::dbg!("obj is {:?}", self);
                        let ptr = obj as *const u64;
                        let v = *ptr;
                        super::super::dbg!("u64 is {}", v);
                        let ptr = ptr.add(1);
                        let v = *(ptr as *const u64);
                        super::super::dbg!("field is {}", v);
                    }

                    match self {
                        #(#fields_writer)*
                    }
                }
            }

            impl WitnessObjReader for #name {
                fn from_witness(&mut self, fetcher: &mut impl FnMut() -> u64,  base: *const u8) {
                    let obj = self as *mut Self;
                    let enum_index = fetcher();
                    unsafe {
                        let ptr = obj as *mut u64;
                        *ptr = enum_index;
                        let obj_ptr = unsafe { ptr.add(1) };
                        match enum_index {
                            #(#fields_reader)*
                            _ => unreachable!()
                        }
                    }
                }
            }
        )
    }

    fn witness_reader(&self) -> Vec<TokenStream2> {
        let mut ret = vec![];
        for i in 0..self.variants.len() {
            let index = i as u64;
            let ty = self.variants[i].ty.clone();
            ret.push(quote!(
                #index => {
                    (*(obj_ptr as *mut #ty)).from_witness(fetcher, base);
                }
            ));
        }
        ret
    }

    fn witness_writer(&self) -> Vec<TokenStream2> {
        let mut ret = vec![];
        for i in 0..self.variants.len() {
            let index = i as u64;
            let name = self.variants[i].name.clone();
            ret.push(quote!(
                Self::#name(obj) => {
                    unsafe { wasm_witness_insert(#index) };
                    obj.to_witness(ori_base);
                }
            ));
        }
        ret
    }
}


#[proc_macro_derive(WitnessObj)]
pub fn derive_witness_obj(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let c = Context::from(input);
    match c {
        Context::S(s) => s.witness_obj_render().into(),
        Context::E(e) => e.witness_obj_render().into()
    }
}
