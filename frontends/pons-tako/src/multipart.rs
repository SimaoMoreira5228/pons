use std::marker::PhantomData;

use pons_core::Schema;
use pons_ir::ContentKind;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tako::extractors::FromRequest;
use tako::extractors::multipart::{FromMultipartField, TakoTypedMultipart, TypedMultipartError};
use tako::types::Request;

use crate::describe::{DescribeRequest, OperationCx};

pub struct PonsMultipart<T, F> {
    pub data: T,
    _file: PhantomData<F>,
}

impl<'a, T, F> FromRequest<'a> for PonsMultipart<T, F>
where
    T: DeserializeOwned + Send + 'static,
    F: FromMultipartField + Serialize + Send + 'static,
{
    type Error = TypedMultipartError;

    async fn from_request(req: &'a mut Request) -> Result<Self, Self::Error> {
        let typed = TakoTypedMultipart::<T, F>::from_request(req).await?;
        Ok(Self {
            data: typed.data,
            _file: PhantomData,
        })
    }
}

impl<T: Schema, F> DescribeRequest for PonsMultipart<T, F> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Multipart);
        cx.diagnostic(
            "PONS-W101",
            format!(
                "the multipart file field type {} has no IR form, so every file part stays opaque",
                std::any::type_name::<F>()
            ),
        );
    }
}
