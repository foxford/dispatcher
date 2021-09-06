use std::{fmt::Debug, time::Duration};

use futures::{
    future::{select_ok, try_select, Either},
    pin_mut, Future,
};

pub async fn single_retry<F, T, E>(
    create_f: impl Fn() -> F,
    retry_timeout: Duration,
) -> Result<T, E>
where
    F: Future<Output = Result<T, E>> + Send,
    E: Debug,
{
    let f = create_f();
    let timeout = async {
        async_std::task::sleep(retry_timeout).await;
        Ok::<(), ()>(())
    };
    pin_mut!(timeout);
    pin_mut!(f);
    let response = try_select(timeout, f).await;
    match response {
        Ok(Either::Left((_, f))) => {
            let next_req = create_f();
            pin_mut!(next_req);
            select_ok([f, next_req]).await.map(|(v, _)| v)
        }
        Ok(Either::Right((x, _))) => Ok(x),
        Err(Either::Left((_, _))) => unreachable!(),
        Err(Either::Right((err, _))) => {
            warn!(crate::LOG, "Request errored: {:?}", err);
            Ok(create_f().await?)
        }
    }
}

#[cfg(test)]
mod tests {
    #[async_std::test]

    async fn should_not_retry() {}
}
