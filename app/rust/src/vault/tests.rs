use super::*;

const PW: &[u8] = b"violet-harbor-tundra-9";

fn fast() -> Argon2Cost {
    Argon2Cost::MINIMUM
}

fn tree(root: &Path) {
    fs::create_dir_all(root.join("proj/src")).unwrap();
    fs::write(root.join("proj/readme.md"), "# hi\n").unwrap();
    fs::write(root.join("proj/src/main.js"), "console.log(1)\n".repeat(50)).unwrap();
    fs::write(root.join("proj/empty.txt"), "").unwrap();
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    fs::write(root.join("proj/big.bin"), big).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(root.join("proj/run.sh"), "#!/bin/sh\necho ok\n").unwrap();
        fs::set_permissions(root.join("proj/run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("readme.md", root.join("proj/link")).unwrap();
    }
}

fn same_tree(a: &Path, b: &Path) {
    let mut names: Vec<_> = fs::read_dir(a)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    for n in names {
        let (pa, pb) = (a.join(&n), b.join(&n));
        let ma = fs::symlink_metadata(&pa).unwrap();
        let mb = fs::symlink_metadata(&pb).unwrap_or_else(|_| panic!("missing {pb:?}"));
        if ma.file_type().is_symlink() {
            assert_eq!(fs::read_link(&pa).unwrap(), fs::read_link(&pb).unwrap());
        } else if ma.is_dir() {
            same_tree(&pa, &pb);
        } else {
            assert_eq!(fs::read(&pa).unwrap(), fs::read(&pb).unwrap(), "{n:?}");
            assert_eq!(is_executable(&ma), is_executable(&mb), "{n:?}");
            assert_eq!(mtime(&ma), mtime(&mb), "{n:?}");
        }
    }
}

#[test]
fn create_import_lock_unlock_export() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    tree(&src);
    let vpath = tmp.path().join("v");
    let (mut v, phrase) = Vault::create(&vpath, PW, fast()).unwrap();
    let mut last = Progress::default();
    let out = v
        .import(&[src.join("proj")], None, false, &mut |p| last = p.clone())
        .unwrap();
    assert_eq!(last.files_done, last.files_total);
    assert_eq!(out.folders, 2);
    drop(v);

    // Nothing in the vault folder reveals a name.
    let all = walk(&vpath);
    for p in &all {
        let s = p.to_string_lossy();
        assert!(
            !s.contains("readme") && !s.contains("main.js") && !s.contains("proj"),
            "{s}"
        );
        let bytes = fs::read(p).unwrap();
        assert!(!bytes.windows(14).any(|w| w == b"console.log(1)"));
    }

    let mut v = Vault::unlock(&vpath, &Unlock::Password(PW)).unwrap();
    let top = v.manifest().children(None);
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].name, "proj");
    let dest = tmp.path().join("out");
    let ids = vec![top[0].id];
    v.export(&ids, &dest, &mut |_| {}).unwrap();
    same_tree(&src.join("proj"), &dest.join("proj"));
    drop(v);

    let key = recovery::from_phrase(&phrase).unwrap();
    let mut v = Vault::unlock(&vpath, &Unlock::Recovery(&key)).unwrap();
    v.change_password(b"another-long-password-42", fast())
        .unwrap();
    drop(v);
    assert!(Vault::unlock(&vpath, &Unlock::Password(PW)).is_err());
    Vault::unlock(&vpath, &Unlock::Password(b"another-long-password-42")).unwrap();
}

#[test]
fn delete_rename_move_and_originals() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    tree(&src);
    let (mut v, _) = Vault::create(&tmp.path().join("v"), PW, fast()).unwrap();
    let out = v
        .import(&[src.join("proj")], None, true, &mut |_| {})
        .unwrap();
    assert!(out.originals_deleted);
    assert!(!src.join("proj").exists());

    let proj = v.manifest().children(None)[0].id;
    let docs = v.create_folder(None, "docs").unwrap();
    let readme = v
        .manifest()
        .children(Some(proj))
        .iter()
        .find(|e| e.name == "readme.md")
        .unwrap()
        .id;
    v.move_to(readme, Some(docs)).unwrap();
    v.rename(readme, "notes.md").unwrap();
    assert!(v.move_to(proj, Some(proj)).is_err());
    let mut text = Vec::new();
    v.read_file(readme, &mut text).unwrap();
    assert_eq!(text, b"# hi\n");

    let blobs_before = walk(&v.path().join(OBJECTS_DIR)).len();
    v.delete(&[proj]).unwrap();
    let blobs_after = walk(&v.path().join(OBJECTS_DIR)).len();
    assert!(blobs_after < blobs_before, "big.bin's own blob is gone");
    // notes.md lives in the shared pack, which must survive.
    let mut again = Vec::new();
    v.read_file(readme, &mut again).unwrap();
    assert_eq!(again, b"# hi\n");
    assert_eq!(v.manifest().children(None).len(), 1);
}

#[test]
fn tampering_is_detected() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    tree(&src);
    let vpath = tmp.path().join("v");
    let (mut v, _) = Vault::create(&vpath, PW, fast()).unwrap();
    v.import(&[src.join("proj/big.bin")], None, false, &mut |_| {})
        .unwrap();
    let id = v.manifest().children(None)[0].id;
    drop(v);
    let blob = walk(&vpath.join(OBJECTS_DIR)).pop().unwrap();
    let mut bytes = fs::read(&blob).unwrap();
    let n = bytes.len() / 2;
    bytes[n] ^= 0x40;
    fs::write(&blob, bytes).unwrap();
    let mut v = Vault::unlock(&vpath, &Unlock::Password(PW)).unwrap();
    assert!(v.read_file(id, &mut Vec::new()).is_err());
}

#[test]
fn refuses_non_empty_folder_and_weak_password() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("x"), "x").unwrap();
    assert!(Vault::create(tmp.path(), PW, fast()).is_err());
    assert!(Vault::create(&tmp.path().join("new"), b"password", fast()).is_err());
}

fn walk(p: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in fs::read_dir(p).unwrap() {
        let e = e.unwrap().path();
        if e.is_dir() {
            v.extend(walk(&e));
        } else {
            v.push(e);
        }
    }
    v
}

fn edit_device_slot(path: &Path, f: impl FnOnce(&mut header::Slot)) {
    let mut h = read_header(path).unwrap();
    let s = h
        .body
        .slots
        .iter_mut()
        .find(|s| s.kind == SlotKind::Device)
        .unwrap();
    f(s);
    fs::write(path.join(HEADER_FILE), h.to_bytes().unwrap()).unwrap();
}

#[test]
fn device_slot_unlocks_until_it_expires() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v");
    let (mut v, _) = Vault::create(&path, PW, fast()).unwrap();
    let dev = b"tablet-1";
    let secret = v.enroll_device(dev, "Tablet").unwrap();
    assert!(v.device_expires(dev).unwrap() > now() + DEVICE_TTL_SECS - 60);
    drop(v);

    let how = Unlock::Device {
        device: dev,
        secret: &secret,
    };
    let v = Vault::unlock(&path, &how).unwrap();
    drop(v);
    // Wrong device id or secret fails.
    assert!(Vault::unlock(
        &path,
        &Unlock::Device {
            device: b"other",
            secret: &secret
        }
    )
    .is_err());
    assert!(Vault::unlock(
        &path,
        &Unlock::Device {
            device: dev,
            secret: &[7u8; 32]
        }
    )
    .is_err());

    // Expired: refused, and editing the expiry by hand breaks the header MAC.
    let (h_before, _) = (read_header(&path).unwrap(), ());
    let expires = h_before.body.device_slot(dev).unwrap().expires.unwrap();
    edit_device_slot(&path, |s| s.expires = Some(now() - 10));
    // The MAC no longer matches, so the edit itself is caught.
    assert!(Vault::unlock(&path, &how).is_err());
    edit_device_slot(&path, |s| s.expires = Some(expires));
    // Restoring the exact value makes the header valid again.
    Vault::unlock(&path, &how).unwrap();

    // A password unlock renews; removing the device turns it off.
    let mut v = Vault::unlock(&path, &Unlock::Password(PW)).unwrap();
    v.renew_device(dev).unwrap();
    v.remove_device(dev).unwrap();
    assert!(v.device_expires(dev).is_none());
    drop(v);
    assert!(Vault::unlock(&path, &how).is_err());
}

#[test]
fn expired_device_slot_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v");
    let (mut v, _) = Vault::create(&path, PW, fast()).unwrap();
    let dev = b"phone";
    let secret = v.enroll_device(dev, "Phone").unwrap();
    // Expire it the legitimate way: through the vault, so the MAC is right.
    let mut body = v.header.body.clone();
    body.device_slot_mut(dev).unwrap().expires = Some(now() - 1);
    v.save_header(body).unwrap();
    drop(v);
    let err = Vault::unlock(
        &path,
        &Unlock::Device {
            device: dev,
            secret: &secret,
        },
    )
    .err()
    .unwrap();
    assert!(format!("{err:?}").contains("expired"));
    // Password still works and renews it.
    let mut v = Vault::unlock(&path, &Unlock::Password(PW)).unwrap();
    v.renew_device(dev).unwrap();
    drop(v);
    Vault::unlock(
        &path,
        &Unlock::Device {
            device: dev,
            secret: &secret,
        },
    )
    .unwrap();
}
