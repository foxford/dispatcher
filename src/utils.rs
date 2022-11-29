use std::{fmt::Debug, time::Duration};

use futures::{
    future::{select_ok, try_select, Either},
    pin_mut, Future,
};
use tracing::warn;

pub async fn single_retry<F, T, E>(
    mut create_f: impl FnMut() -> F,
    retry_delay: Duration,
) -> Result<T, E>
where
    F: Future<Output = Result<T, E>> + Send,
    E: Debug,
{
    let f = create_f();
    let timeout = async {
        tokio::time::sleep(retry_delay).await;
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
            warn!("Request errored: {:?}", err);
            Ok(create_f().await?)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use super::single_retry;

    #[tokio::test]
    async fn should_retry_when_failed() {
        let call_count = Arc::new(Mutex::new(0));
        let create_fut = || {
            let call_count = call_count.clone();
            async move {
                let mut mutex_guard = call_count.lock().unwrap();
                *mutex_guard += 1;
                if *mutex_guard == 1 {
                    Err(())
                } else {
                    Ok(())
                }
            }
        };

        let result = single_retry(create_fut, Duration::from_secs(10000)).await;

        let guard = call_count.lock().unwrap();
        assert_eq!(*guard, 2);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn should_return_second_error_when_all_failed() {
        let call_count = Arc::new(Mutex::new(0));
        let create_fut = || {
            let call_count = call_count.clone();
            async move {
                let mut mutex_guard = call_count.lock().unwrap();
                *mutex_guard += 1;
                if *mutex_guard == 1 {
                    Err::<(), _>(*mutex_guard)
                } else {
                    Err(*mutex_guard)
                }
            }
        };

        let result = single_retry(create_fut, Duration::from_secs(10000)).await;

        let guard = call_count.lock().unwrap();
        assert_eq!(*guard, 2);
        assert_eq!(result, Err(2));
    }

    #[tokio::test]
    async fn should_retry_after_delay() {
        let call_count = Arc::new(Mutex::new(0));
        let create_fut = || {
            let call_count = call_count.clone();
            async move {
                let call_count = {
                    let mut mutex_guard = call_count.lock().unwrap();
                    *mutex_guard += 1;
                    *mutex_guard
                };
                if call_count == 1 {
                    tokio::time::sleep(Duration::from_secs(10000000)).await;
                    Ok::<_, ()>(())
                } else {
                    Ok(())
                }
            }
        };

        let result = single_retry(create_fut, Duration::from_millis(1)).await;

        let guard = call_count.lock().unwrap();
        assert_eq!(*guard, 2);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn should_not_retry_when_ok() {
        let call_count = Arc::new(Mutex::new(0));
        let create_fut = || {
            let call_count = call_count.clone();
            async move {
                let call_count = {
                    let mut mutex_guard = call_count.lock().unwrap();
                    *mutex_guard += 1;
                    *mutex_guard
                };
                if call_count == 1 {
                    Ok::<_, ()>(())
                } else {
                    Ok(())
                }
            }
        };

        let result = single_retry(create_fut, Duration::from_millis(1)).await;

        let guard = call_count.lock().unwrap();
        assert_eq!(*guard, 1);
        assert!(result.is_ok());
    }
}
