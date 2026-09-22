//! `fetch()` binding: a synchronous HTTP request behind the JS-side
//! Promise wrapper in `bootstrap.js`, gated by CSP `connect-src`.

use oasis_js::rquickjs::{Ctx, Function, Result as JsResult};

/// Install `__oasis_fetch(method, url, body) -> String`.
pub(super) fn install_fetch_binding(
    ctx: &Ctx<'_>,
    url: &str,
    csp: Option<&crate::loader::csp::CspPolicy>,
) -> JsResult<()> {
    let globals = ctx.globals();

    // -- __oasis_fetch(method, url, body) -> String ----------------------
    {
        let fetch_csp = csp.cloned();
        let fetch_page_url = url.to_string();
        globals.set(
            "__oasis_fetch",
            Function::new(
                ctx.clone(),
                move |method: String, url_str: String, body_str: String| -> String {
                    // Enforce CSP connect-src before making the request.
                    if let Some(ref policy) = fetch_csp
                        && policy.is_active()
                        && !policy.allows(
                            &url_str,
                            &fetch_page_url,
                            crate::loader::csp::CspResourceType::Connect,
                        )
                    {
                        return String::new();
                    }
                    let body_bytes = if body_str.is_empty() {
                        None
                    } else {
                        Some(body_str.as_bytes())
                    };
                    match crate::loader::Url::parse(&url_str) {
                        Some(parsed_url) => {
                            match crate::loader::http::http_request(
                                &method,
                                &parsed_url,
                                body_bytes,
                                &[],
                                None,
                            ) {
                                Ok(resp) => String::from_utf8_lossy(&resp.body).into_owned(),
                                Err(_) => String::new(),
                            }
                        },
                        None => String::new(),
                    }
                },
            )?,
        )?;
    }

    Ok(())
}
