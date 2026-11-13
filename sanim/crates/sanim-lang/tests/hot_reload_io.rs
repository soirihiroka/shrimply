use sanim_lang::{Reload,Session,Sources};
use std::{cell::RefCell,collections::BTreeMap};

#[derive(Default)]
struct CountingSources {
    files:RefCell<BTreeMap<String,Vec<u8>>>,
    reads:RefCell<BTreeMap<String,usize>>,
}
impl CountingSources {
    fn insert(&self,path:&str,text:&str) {self.files.borrow_mut().insert(path.into(),text.as_bytes().to_vec());}
    fn count(&self,path:&str)->usize {*self.reads.borrow().get(path).unwrap_or(&0)}
}
impl Sources for CountingSources {
    fn read(&self,path:&str)->Result<Vec<u8>,String> {
        *self.reads.borrow_mut().entry(path.into()).or_default()+=1;
        self.files.borrow().get(path).cloned().ok_or_else(||format!("missing {path}"))
    }
}

#[test]
fn changed_reflected_input_does_not_probe_a_later_obsolete_resource() {
    let files=CountingSources::default();
    files.insert("main.sanim","let enable=reflect(true,key=\"enabled\")\nif enable {\nlet text=read<String>(\"later.txt\")\nif text!=\"\" {wait(1s)}\n}\nwait(1s)\n");
    files.insert("later.txt","expensive data");
    let mut session=Session::new("main.sanim");
    assert!(matches!(session.poll(&files),Reload::Updated{..}));
    assert!(files.count("later.txt")>0);
    files.reads.borrow_mut().clear();
    files.files.borrow_mut().remove("later.txt");
    session.set_json_inputs(BTreeMap::from([("enabled".into(),serde_json::json!(false))]));
    let Reload::Updated {prepared,..}=session.poll(&files) else {panic!("disabling the read must not fail on its obsolete dependency")};
    assert_eq!(files.count("later.txt"),0,"obsolete data must not be loaded, hashed or fetched before invalidating its old execution prefix");
    assert!(!prepared.dependencies.contains_key("later.txt"));
    assert_eq!(prepared.animation.duration,1.into());
}

#[test]
fn changed_function_source_does_not_probe_its_old_resource() {
    let files=CountingSources::default();
    files.insert("main.sanim","fn hold():Duration {\nlet text=read<String>(\"later.txt\")\nif text!=\"\" {return 2s}\nreturn 1s\n}\nwait(hold())\n");
    files.insert("later.txt","expensive data");
    let mut session=Session::new("main.sanim");
    assert!(matches!(session.poll(&files),Reload::Updated{..}));
    files.reads.borrow_mut().clear();
    files.files.borrow_mut().remove("later.txt");
    files.insert("main.sanim","fn hold():Duration {return 3s}\nwait(hold())\n");
    let Reload::Updated {prepared,..}=session.poll(&files) else {panic!("the new function no longer reads the deleted resource")};
    assert_eq!(files.count("later.txt"),0);
    assert_eq!(prepared.animation.duration,3.into());
    assert!(!prepared.dependencies.contains_key("later.txt"));
}
