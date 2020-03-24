use crate::{clear::Clear, tests::util::*, Pool};
use loom::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
};

#[derive(Default, Debug)]
struct State {
    is_dropped: AtomicBool,
    is_cleared: AtomicBool,
    id: usize,
}

impl PartialEq for State {
    fn eq(&self, other: &State) -> bool {
        self.id.eq(&other.id)
    }
}

#[derive(Default, Debug)]
struct DontDropMe(Arc<State>);

impl PartialEq for DontDropMe {
    fn eq(&self, other: &DontDropMe) -> bool {
        self.0.eq(&other.0)
    }
}

impl DontDropMe {
    fn new(id: usize) -> (Arc<State>, Self) {
        let state = Arc::new(State {
            is_dropped: AtomicBool::new(false),
            is_cleared: AtomicBool::new(false),
            id,
        });
        (state.clone(), Self(state))
    }
}

impl Drop for DontDropMe {
    fn drop(&mut self) {
        test_println!("-> DontDropMe drop: dropping data {:?}", self.0.id);
        self.0.is_dropped.store(true, Ordering::SeqCst)
    }
}

impl Clear for DontDropMe {
    fn clear(&mut self) {
        test_println!("-> DontDropMe clear: clearing data {:?}", self.0.id);
        self.0.is_cleared.store(true, Ordering::SeqCst);
    }
}

#[test]
fn pool_dont_drop() {
    run_model("pool_dont_drop", || {
        let pool: Pool<DontDropMe> = Pool::new();
        let (item1, value) = DontDropMe::new(1);
        test_println!("-> dont_drop: Inserting into pool {}", item1.id);
        let mut value = Some(value);
        let idx = pool
            .create(move |item| *item = value.take().expect("Value created twice"))
            .expect("Create");

        test_println!("-> dont_drop: clearing idx: {}", idx);
        pool.clear(idx);

        assert!(!item1.is_dropped.load(Ordering::SeqCst));
        assert!(item1.is_cleared.load(Ordering::SeqCst));
    });
}

#[test]
fn pool_concurrent_create_clear() {
    run_model("pool_concurrent_create_clear", || {
        let pool: Arc<Pool<DontDropMe>> = Arc::new(Pool::new());
        let pair = Arc::new((Mutex::new(None), Condvar::new()));

        let (item1, value) = DontDropMe::new(1);

        let mut value = Some(value);
        let idx1 = pool
            .create(move |item| *item = value.take().expect("value created twice"))
            .expect("Create");

        let p = pool.clone();
        let pair2 = pair.clone();
        let test_value = item1.clone();
        let t1 = thread::spawn(move || {
            let (lock, cvar) = &*pair2;
            assert_eq!(p.get(idx1).unwrap().0.id, test_value.id);
            let mut next = lock.lock().unwrap();
            *next = Some(());
            cvar.notify_one();
        });

        let guard = pool.get(idx1);

        let (lock, cvar) = &*pair;
        let mut next = lock.lock().unwrap();
        // wait until we have a guard on the other thread.
        while next.is_none() {
            next = cvar.wait(next).unwrap();
        }
        assert!(!pool.clear(idx1));

        assert!(!item1.is_dropped.load(Ordering::SeqCst));
        assert!(!item1.is_cleared.load(Ordering::SeqCst));

        assert_eq!(guard.unwrap().0.id, item1.id);

        t1.join().expect("thread 1 unable to join");

        assert!(!item1.is_dropped.load(Ordering::SeqCst));
        assert!(item1.is_cleared.load(Ordering::SeqCst));
    })
}

#[test]
fn pool_racy_clear() {
    run_model("pool_racy_clear", || {
        let pool = Arc::new(Pool::new());
        let (item, value) = DontDropMe::new(1);

        let mut value = Some(value);
        let idx = pool
            .create(move |item| *item = value.take().expect("value created twice"))
            .expect("Create");
        assert_eq!(pool.get(idx).unwrap().0.id, item.id);

        let p = pool.clone();
        let t2 = thread::spawn(move || p.clear(idx));
        let r1 = pool.clear(idx);
        let r2 = t2.join().expect("thread 2 should not panic");

        test_println!("r1: {}, r2: {}", r1, r2);

        assert!(
            !(r1 && r2),
            "Both threads should not have cleared the value"
        );
        assert!(r1 || r2, "One thread should have removed the value");
        assert!(pool.get(idx).is_none());
        assert!(!item.is_dropped.load(Ordering::SeqCst));
        assert!(item.is_cleared.load(Ordering::SeqCst));
    })
}
