use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token};

#[derive(Clone, Copy)]
pub(crate) enum Method {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

impl Method {
    pub(crate) fn from_ident(ident: &Ident) -> syn::Result<Self> {
        match ident.to_string().to_ascii_uppercase().as_str() {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "PUT" => Ok(Self::Put),
            "DELETE" => Ok(Self::Delete),
            "PATCH" => Ok(Self::Patch),
            "HEAD" => Ok(Self::Head),
            "OPTIONS" => Ok(Self::Options),
            other => Err(syn::Error::new(
                ident.span(),
                format!(
                    "unknown HTTP method `{other}`; expected GET, POST, PUT, DELETE, PATCH, HEAD, or OPTIONS"
                ),
            )),
        }
    }

    pub(crate) fn http_name(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Patch => "PATCH",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }

    pub(crate) fn router_method(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Put => "put",
            Self::Delete => "delete",
            Self::Patch => "patch",
            Self::Head => "head",
            Self::Options => "options",
        }
    }
}

pub(crate) struct ShortcutArgs {
    pub(crate) path: LitStr,
    pub(crate) name: Option<LitStr>,
}

impl Parse for ShortcutArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let path = parse_path(input)?;
        let name = parse_optional_name(input)?;
        parse_trailing_comma(input)?;
        Ok(Self { path, name })
    }
}

pub(crate) struct RouteArgs {
    pub(crate) method: Method,
    pub(crate) path: LitStr,
    pub(crate) name: Option<LitStr>,
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let method: Ident = input
            .parse()
            .map_err(|error| syn::Error::new(error.span(), "expected an HTTP method"))?;
        let method = Method::from_ident(&method)?;
        input.parse::<Token![,]>()?;
        let path = parse_path(input)?;
        let name = parse_optional_name(input)?;
        parse_trailing_comma(input)?;
        Ok(Self { method, path, name })
    }
}

fn parse_path(input: ParseStream) -> syn::Result<LitStr> {
    input
        .parse()
        .map_err(|error| syn::Error::new(error.span(), "the route path must be a string literal"))
}

fn parse_optional_name(input: ParseStream) -> syn::Result<Option<LitStr>> {
    if input.is_empty() {
        return Ok(None);
    }
    input.parse::<Token![,]>()?;
    if input.is_empty() {
        return Ok(None);
    }
    let key: Ident = input.parse()?;
    if key != "name" {
        return Err(syn::Error::new(key.span(), "expected `name = \"...\"`"));
    }
    input.parse::<Token![=]>()?;
    let name: LitStr = input
        .parse()
        .map_err(|error| syn::Error::new(error.span(), "route names must be string literals"))?;
    Ok(Some(name))
}

fn parse_trailing_comma(input: ParseStream) -> syn::Result<()> {
    input.parse::<Option<Token![,]>>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn a_non_string_path_points_at_the_value() {
        let Err(error) = syn::parse2::<ShortcutArgs>(quote!(42)) else {
            panic!("a non-string path was accepted");
        };
        assert_eq!(error.to_string(), "the route path must be a string literal");
    }

    #[test]
    fn an_unknown_method_points_at_the_method() {
        let Err(error) = syn::parse2::<RouteArgs>(quote!(TRACE, "/users")) else {
            panic!("an unknown method was accepted");
        };
        assert_eq!(
            error.to_string(),
            "unknown HTTP method `TRACE`; expected GET, POST, PUT, DELETE, PATCH, HEAD, or OPTIONS"
        );
    }

    #[test]
    fn methods_accept_either_case() {
        for (input, expected) in [
            (quote!(get), "GET"),
            (quote!(POST), "POST"),
            (quote!(Patch), "PATCH"),
            (quote!(options), "OPTIONS"),
        ] {
            let args = syn::parse2::<RouteArgs>(quote!(#input, "/users")).unwrap();
            assert_eq!(args.method.http_name(), expected);
        }
    }

    #[test]
    fn the_name_is_optional_and_tolerates_a_trailing_comma() {
        let args = syn::parse2::<ShortcutArgs>(quote!("/users/{id}", name = "users.get",)).unwrap();
        assert_eq!(args.path.value(), "/users/{id}");
        assert_eq!(args.name.unwrap().value(), "users.get");

        let args = syn::parse2::<ShortcutArgs>(quote!("/users",)).unwrap();
        assert!(args.name.is_none());
    }
}
