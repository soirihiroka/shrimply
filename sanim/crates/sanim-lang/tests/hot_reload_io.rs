use sanim_lang::{Reload, Session, Sources};
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Default)]
struct CountingSources {
    files: RefCell<BTreeMap<String, Vec<u8>>>,
    reads: RefCell<BTreeMap<String, usize>>,
}
impl CountingSources {
    fn insert(&self, path: &str, text: &str) {
        self.files
            .borrow_mut()
            .insert(path.into(), text.as_bytes().to_vec());
    }
    fn count(&self, path: &str) -> usize {
        *self.reads.borrow().get(path).unwrap_or(&0)
    }
}
impl Sources for CountingSources {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        *self.reads.borrow_mut().entry(path.into()).or_default() += 1;
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing {path}"))
    }
}

#[test]
fn changed_reflected_input_does_not_probe_a_later_obsolete_resource() {
    let files = CountingSources::default();
    files.insert("main.sanim","let enable=reflect(true,key=\"enabled\")\nif enable {\nlet text=read<String>(\"later.txt\")\nif text!=\"\" {wait(1s)}\n}\nwait(1s)\n");
    files.insert("later.txt", "expensive data");
    let mut session = Session::new("main.sanim");
    assert!(matches!(session.poll(&files), Reload::Updated { .. }));
    assert!(files.count("later.txt") > 0);
    files.reads.borrow_mut().clear();
    files.files.borrow_mut().remove("later.txt");
    session.set_json_inputs(BTreeMap::from([(
        "enabled".into(),
        serde_json::json!(false),
    )]));
    let Reload::Updated { prepared, .. } = session.poll(&files) else {
        panic!("disabling the read must not fail on its obsolete dependency")
    };
    assert_eq!(
        files.count("later.txt"),
        0,
        "obsolete data must not be loaded, hashed or fetched before invalidating its old execution prefix"
    );
    assert!(!prepared.dependencies.contains_key("later.txt"));
    assert_eq!(prepared.animation.duration, 1.into());
}

#[test]
fn changed_function_source_does_not_probe_its_old_resource() {
    let files = CountingSources::default();
    files.insert("main.sanim","fn hold():Duration {\nlet text=read<String>(\"later.txt\")\nif text!=\"\" {return 2s}\nreturn 1s\n}\nwait(hold())\n");
    files.insert("later.txt", "expensive data");
    let mut session = Session::new("main.sanim");
    assert!(matches!(session.poll(&files), Reload::Updated { .. }));
    files.reads.borrow_mut().clear();
    files.files.borrow_mut().remove("later.txt");
    files.insert(
        "main.sanim",
        "fn hold():Duration {return 3s}\nwait(hold())\n",
    );
    let Reload::Updated { prepared, .. } = session.poll(&files) else {
        panic!("the new function no longer reads the deleted resource")
    };
    assert_eq!(files.count("later.txt"), 0);
    assert_eq!(prepared.animation.duration, 3.into());
    assert!(!prepared.dependencies.contains_key("later.txt"));
}

#[test]
fn progress_preserves_snapshot_providers_and_hot_duration_does_not_reexecute() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Snapshots {
        main: RefCell<Arc<Vec<u8>>>,
        payload: Arc<Vec<u8>>,
        payload_hash: String,
    }
    impl Sources for Snapshots {
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            Err(format!("snapshot provider must not be bypassed for {path}"))
        }
        fn read_snapshot(&self, path: &str) -> Result<(Arc<Vec<u8>>, String), String> {
            match path {
                "main.sanim" => {
                    let bytes = self.main.borrow().clone();
                    let hash = sanim_core::digest(&bytes);
                    Ok((bytes, hash))
                }
                "payload.txt" => Ok((self.payload.clone(), self.payload_hash.clone())),
                _ => Err(format!("missing {path}")),
            }
        }
    }
    let source = |wait: &str| {
        Arc::new(format!("let payload=read<String>(\"payload.txt\")\nif payload!=\"\" {{add(Circle())}}\nwait({wait})").into_bytes())
    };
    let payload = Arc::new(vec![b'x'; 32 * 1024]);
    let files = Snapshots {
        main: RefCell::new(source("3s")),
        payload_hash: sanim_core::digest(&payload),
        payload,
    };
    let completed = Arc::new(AtomicUsize::new(0));
    let mut session = Session::new("main.sanim");
    session.engine.set_progress_observer(Some(Arc::new({
        let completed = completed.clone();
        move |progress| {
            if progress.state == sanim_lang::progress::State::Ready {
                completed.fetch_add(1, Ordering::Relaxed);
            }
        }
    })));
    let mut duration = None;
    assert!(matches!(
        session.poll_with_duration(&files, |value| duration = Some(value)),
        Reload::Updated { .. }
    ));
    assert_eq!(duration, Some(3.into()));
    assert_eq!(
        completed.swap(0, Ordering::Relaxed),
        2,
        "cold preparation retains its accurate duration preflight"
    );
    *files.main.borrow_mut() = source("4s");
    let Reload::Updated { prepared, .. } =
        session.poll_with_duration(&files, |value| duration = Some(value))
    else {
        panic!("hot revision")
    };
    assert_eq!(duration, Some(4.into()));
    assert_eq!(prepared.animation.duration, 4.into());
    assert!(prepared.statistics.restored_statements >= 2);
    assert_eq!(
        completed.load(Ordering::Relaxed),
        1,
        "hot edits restore once rather than replaying the duration program first"
    );
}
