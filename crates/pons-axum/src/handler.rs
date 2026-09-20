use std::future::Future;

use crate::describe::{DescribeRequest, DescribeResponse, OperationCx};

#[doc(hidden)]
pub trait DescribeHandler<T, S> {
    fn describe(cx: &mut OperationCx);
}

impl<F, Fut, Res, S> DescribeHandler<((),), S> for F
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Res>,
    Res: DescribeResponse,
{
    fn describe(cx: &mut OperationCx) {
        Res::describe(cx);
    }
}

macro_rules! impl_describe_handler {
    ([$($arg:ident),*], $last:ident) => {
        impl<HandlerFn, Fut, Res, State, Mode, $($arg,)* $last> DescribeHandler<(Mode, $($arg,)* $last,), State> for HandlerFn
        where
            HandlerFn: FnOnce($($arg,)* $last,) -> Fut,
            Fut: Future<Output = Res>,
            $($arg: DescribeRequest,)*
            $last: DescribeRequest,
            Res: DescribeResponse,
        {
            fn describe(cx: &mut OperationCx) {
                $($arg::describe(cx);)*
                $last::describe(cx);
                Res::describe(cx);
            }
        }
    };
}

impl_describe_handler!([], T1);
impl_describe_handler!([T1], T2);
impl_describe_handler!([T1, T2], T3);
impl_describe_handler!([T1, T2, T3], T4);
impl_describe_handler!([T1, T2, T3, T4], T5);
impl_describe_handler!([T1, T2, T3, T4, T5], T6);
impl_describe_handler!([T1, T2, T3, T4, T5, T6], T7);
impl_describe_handler!([T1, T2, T3, T4, T5, T6, T7], T8);
impl_describe_handler!([T1, T2, T3, T4, T5, T6, T7, T8], T9);
impl_describe_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9], T10);
impl_describe_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10], T11);
impl_describe_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11], T12);
impl_describe_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12], T13);
impl_describe_handler!(
    [T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13],
    T14
);
impl_describe_handler!(
    [T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14],
    T15
);
impl_describe_handler!(
    [
        T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15
    ],
    T16
);
