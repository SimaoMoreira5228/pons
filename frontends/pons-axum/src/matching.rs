use pons_ir::{Document, Method, PathSegment};

pub(crate) fn declared_media_type(
    document: &Document,
    request: &axum::extract::Request,
    default: &str,
) -> String {
    let Some(method) = method(request.method()) else {
        return default.to_owned();
    };
    document
        .operations
        .iter()
        .filter(|operation| {
            operation.method == method && path_matches(&operation.path, request.uri().path())
        })
        .max_by_key(|operation| specificity(&operation.path))
        .and_then(|operation| operation.body.as_ref()?.media.as_ref())
        .map(ToString::to_string)
        .unwrap_or_else(|| default.to_owned())
}

fn specificity(segments: &[PathSegment]) -> (usize, Vec<u8>) {
    (
        segments.len(),
        segments
            .iter()
            .map(|segment| match segment {
                PathSegment::Literal { .. } => 1,
                PathSegment::Param { .. } => 0,
            })
            .collect(),
    )
}

fn method(method: &axum::http::Method) -> Option<Method> {
    Some(match *method {
        axum::http::Method::GET => Method::Get,
        axum::http::Method::POST => Method::Post,
        axum::http::Method::PUT => Method::Put,
        axum::http::Method::PATCH => Method::Patch,
        axum::http::Method::DELETE => Method::Delete,
        axum::http::Method::HEAD => Method::Head,
        axum::http::Method::OPTIONS => Method::Options,
        _ => return None,
    })
}

fn path_matches(segments: &[PathSegment], path: &str) -> bool {
    let mut parts = path_segments(path).into_iter();
    segments
        .iter()
        .enumerate()
        .all(|(index, segment)| match (segment, parts.next()) {
            (PathSegment::Param { .. }, Some(part)) => {
                index + 1 < segments.len() || !part.is_empty()
            }
            (PathSegment::Literal { lit }, Some(part)) => lit == part,
            _ => false,
        })
        && parts.next().is_none()
}

fn path_segments(path: &str) -> Vec<&str> {
    match path.strip_prefix('/').unwrap_or(path) {
        "" => Vec::new(),
        rest => rest.split('/').collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pons_ir::{InlineType, TypeRef};

    fn segments(path: &str) -> Vec<PathSegment> {
        path.split('/')
            .map(|segment| match segment.strip_prefix('{') {
                Some(param) => PathSegment::Param {
                    param: param.trim_end_matches('}').to_owned(),
                    ty: TypeRef::Inline(InlineType::String {
                        format: None,
                        min_length: None,
                        max_length: None,
                        pattern: None,
                    }),
                },
                None => PathSegment::Literal {
                    lit: segment.to_owned(),
                },
            })
            .collect()
    }

    #[test]
    fn a_path_parameter_matches_one_segment_and_no_more() {
        let assets = segments("assets/{id}");
        assert!(path_matches(&assets, "/assets/1"));
        assert!(
            !path_matches(&assets, "/assets/"),
            "matchit ends the search when the path runs out, so a trailing parameter never binds the empty segment axum leaves after a slash"
        );
        assert!(
            !path_matches(&assets, "/assets//"),
            "the empty segment axum leaves after a slash is a segment of its own"
        );
        assert!(
            !path_matches(&assets, "/assets/1/"),
            "an empty segment is a segment axum serves, so /assets/1/ is another route"
        );
        assert!(
            !path_matches(&assets, "/assets/1/raw"),
            "a parameter covers one segment, so a deeper path is another operation's"
        );
        assert!(!path_matches(&assets, "/assets"));
        assert!(!path_matches(&assets, "/other/1"));
        assert!(
            path_matches(&[], "/"),
            "the root operation matches only the root"
        );

        let raw = segments("assets/{id}/raw");
        assert!(path_matches(&raw, "/assets/1/raw"));
        assert!(!path_matches(&raw, "/assets/1"));
        assert!(
            path_matches(&raw, "/assets//raw"),
            "an interior parameter binds the empty segment between two slashes, which is a segment axum serves"
        );

        let directory = segments("assets/");
        assert!(path_matches(&directory, "/assets/"));
        assert!(
            !path_matches(&directory, "/assets"),
            "/assets and /assets/ are two routes, and this one is the second"
        );
    }

    #[test]
    fn the_most_specific_match_wins_the_resolution() {
        let parameter = segments("files/{id}");
        let trailing = segments("files/{id}/");
        assert!(
            specificity(&trailing) > specificity(&parameter),
            "the longer path is the one a client reaches by naming the trailing segment"
        );
        assert!(
            specificity(&segments("files/raw")) > specificity(&parameter),
            "a literal segment is more specific than a parameter, which is the rule matchit applies"
        );
        assert!(
            specificity(&segments("files/raw")) > specificity(&segments("{a}/raw")),
            "the first segment that differs decides, so a literal earlier in the path wins"
        );
    }

    #[test]
    fn every_published_method_is_read_and_no_other_is_folded_onto_one() {
        for (sent, expected) in [
            ("GET", Method::Get),
            ("POST", Method::Post),
            ("PUT", Method::Put),
            ("PATCH", Method::Patch),
            ("DELETE", Method::Delete),
            ("HEAD", Method::Head),
            ("OPTIONS", Method::Options),
        ] {
            let sent = axum::http::Method::from_bytes(sent.as_bytes()).unwrap();
            assert_eq!(method(&sent), Some(expected), "{sent} is published");
        }
        for sent in [
            axum::http::Method::CONNECT,
            axum::http::Method::TRACE,
            axum::http::Method::from_bytes(b"PROPFIND").unwrap(),
        ] {
            assert_eq!(
                method(&sent),
                None,
                "{sent} is routable but no operation can publish it, so it reads no operation's body"
            );
        }
    }
}
