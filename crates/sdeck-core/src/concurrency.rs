use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

/// Runs `f` over `items` with at most `limit` calls in flight, preserving input order. A worker pool
/// over a shared index cursor, not fixed-size batching: a worker takes the next pending item as soon
/// as it is free. A panic in `f` propagates to the caller.
pub fn map_with_concurrency<T, R, F>(items: &[T], limit: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T, usize) -> R + Sync,
{
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    let next = AtomicUsize::new(0);
    let workers = limit.min(items.len()).max(1);

    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= items.len() {
                    return;
                }
                let value = f(&items[i], i);
                results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(value);
            });
        }
    });

    results
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .map(|r| r.expect("every item was processed"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn preserves_input_order_regardless_of_completion_order() {
        let delays = [30u64, 10, 20, 0];
        let result = map_with_concurrency(&delays, 4, |ms, _| {
            thread::sleep(Duration::from_millis(*ms));
            *ms
        });
        assert_eq!(result, delays);
    }

    #[test]
    fn never_runs_more_than_limit_calls_at_once_and_reaches_it() {
        let items: Vec<usize> = (0..10).collect();
        let in_flight = AtomicUsize::new(0);
        let max_in_flight = AtomicUsize::new(0);
        map_with_concurrency(&items, 3, |item, _| {
            let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            max_in_flight.fetch_max(now, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(20));
            in_flight.fetch_sub(1, Ordering::SeqCst);
            *item
        });
        assert_eq!(max_in_flight.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_limit_larger_than_the_item_count_still_works() {
        assert_eq!(map_with_concurrency(&[1, 2, 3], 100, |n, _| n * 2), vec![2, 4, 6]);
    }

    #[test]
    fn handles_an_empty_input() {
        let items: [i32; 0] = [];
        let result: Vec<i32> = map_with_concurrency(&items, 5, |_, _| panic!("never called"));
        assert!(result.is_empty());
    }

    #[test]
    fn propagates_a_panic() {
        let outcome = std::panic::catch_unwind(|| {
            map_with_concurrency(&[1, 2, 3], 2, |n, _| {
                if *n == 2 {
                    panic!("boom");
                }
                *n
            })
        });
        assert!(outcome.is_err());
    }

    #[test]
    fn starts_the_next_item_as_soon_as_a_slot_frees_up() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let order = Mutex::new(Vec::new());
        thread::scope(|s| {
            let handle = s.spawn(|| {
                map_with_concurrency(&[0, 1, 2], 2, |n, _| {
                    if *n == 0 {
                        release_rx.lock().unwrap().recv().unwrap();
                    }
                    order.lock().unwrap().push(*n);
                    *n
                })
            });
            for _ in 0..200 {
                if order.lock().unwrap().len() == 2 {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(*order.lock().unwrap(), vec![1, 2]);
            release_tx.send(()).unwrap();
            handle.join().unwrap();
        });
        assert_eq!(*order.lock().unwrap(), vec![1, 2, 0]);
    }
}
