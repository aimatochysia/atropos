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

#[test]
fn card_slot_round_trip_with_simulated_card() {
    use crate::card::sim::{block_on, SimCard};
    use crate::card::{agree, enroll_ephemeral, probe, Curve};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v");
    let (mut v, _) = Vault::create(&path, PW, fast()).unwrap();
    let mut card = SimCard::piv(Curve::P384);
    let key = block_on(probe(&mut card)).unwrap().key.unwrap();
    let (ephemeral, shared) = enroll_ephemeral(&key).unwrap();
    let slot = CardSlotJson {
        applet: key.applet.as_str().into(),
        curve: key.curve.as_str().into(),
        public: key.public.clone(),
        ephemeral,
    };
    v.add_card(slot.clone(), &shared, "Work card").unwrap();
    assert_eq!(v.cards().len(), 1);
    drop(v);

    assert_eq!(Vault::card_slots(&path).unwrap(), vec![slot.clone()]);
    let on_card = block_on(agree(&mut card, &key, b"123456", &slot.ephemeral)).unwrap();
    let mut v = Vault::unlock(
        &path,
        &Unlock::Card {
            card: &slot,
            shared: &on_card,
        },
    )
    .unwrap();
    // Another card's answer does not open it.
    let mut other = SimCard::piv(Curve::P384);
    let other_key = block_on(probe(&mut other)).unwrap().key.unwrap();
    let wrong = block_on(agree(&mut other, &other_key, b"123456", &slot.ephemeral)).unwrap();
    assert!(Vault::unlock(
        &path,
        &Unlock::Card {
            card: &slot,
            shared: &wrong
        }
    )
    .is_err());
    let id = v.cards()[0].0.clone();
    v.remove_card(&id).unwrap();
    assert!(v.cards().is_empty());
}

/// Makes `path` look `secs` seconds old.
fn age(path: &Path, secs: i64) {
    set_mtime(path, now() - secs);
}

fn all_files(dir: &Path) -> Vec<PathBuf> {
    walk(dir).into_iter().filter(|p| p.is_file()).collect()
}

#[test]
fn check_finds_and_repair_fixes_crash_leftovers() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    tree(&src);
    let vpath = tmp.path().join("v");
    let (mut v, _) = Vault::create(&vpath, PW, fast()).unwrap();
    v.import(&[src.join("proj")], None, false, &mut |_| {})
        .unwrap();

    let clean = v.check(&mut |_| {}).unwrap();
    assert!(clean.is_clean(), "{clean:?}");
    assert!(clean.files_checked >= 3);

    // An import that crashed: a blob written but never listed, and temp
    // files from interrupted writes. Fresh ones may belong to another
    // device still syncing, so they are left alone until they are old.
    let orphan = blob::write_blob(
        &vpath.join(OBJECTS_DIR),
        v.vault_id(),
        &v.public_key().unwrap(),
        &b"never listed"[..],
        12,
    )
    .unwrap();
    let orphan_path = blob::blob_path(&vpath.join(OBJECTS_DIR), &orphan);
    let seg_tmp = vpath
        .join(MANIFEST_DIR)
        .join("0000000000000099-deadbeef.tmp");
    fs::write(&seg_tmp, b"half written").unwrap();
    let r = v.check(&mut |_| {}).unwrap();
    assert!(r.is_clean(), "fresh files are not leftovers yet: {r:?}");

    age(&orphan_path, 2 * 24 * 3600);
    age(&seg_tmp, 2 * 3600);
    let r = v.check(&mut |_| {}).unwrap();
    assert_eq!(r.orphans, vec![orphan_path.clone()]);
    assert_eq!(r.leftovers, vec![seg_tmp.clone()]);
    let fixed = v.repair(false).unwrap();
    assert_eq!(fixed.deleted_orphans, 1);
    assert_eq!(fixed.deleted_leftovers, 1);
    assert!(!orphan_path.exists() && !seg_tmp.exists());
    assert!(v.check(&mut |_| {}).unwrap().is_clean());
    // Repair needs a fresh check each time.
    v.repair(false).unwrap();
    assert!(v.repair(false).is_err());
}

#[test]
fn check_reports_damaged_and_missing_files() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    tree(&src);
    let vpath = tmp.path().join("v");
    let (mut v, _) = Vault::create(&vpath, PW, fast()).unwrap();
    v.import(&[src.join("proj")], None, false, &mut |_| {})
        .unwrap();
    let blobs: Vec<PathBuf> = all_files(&vpath.join(OBJECTS_DIR));
    assert!(blobs.len() >= 2);
    // Flip a byte in one blob and delete another.
    let mut bytes = fs::read(&blobs[0]).unwrap();
    let n = bytes.len();
    bytes[n - 20] ^= 1;
    fs::write(&blobs[0], bytes).unwrap();
    fs::remove_file(&blobs[1]).unwrap();
    drop(v);
    let mut v = Vault::unlock(&vpath, &Unlock::Password(PW)).unwrap();
    let r = v.check(&mut |_| {}).unwrap();
    assert!(!r.damaged.is_empty(), "{r:?}");
    assert!(!r.missing.is_empty(), "{r:?}");
    let before = v.manifest().entries.len();

    // Without permission nothing is taken off the list.
    v.repair(false).unwrap();
    assert_eq!(v.manifest().entries.len(), before);

    v.check(&mut |_| {}).unwrap();
    let fixed = v.repair(true).unwrap();
    assert_eq!(
        fixed.removed_entries as usize,
        r.damaged.len() + r.missing.len()
    );
    assert!(!blobs[0].exists(), "the damaged blob is deleted");
    assert!(v.check(&mut |_| {}).unwrap().is_clean());
}

#[test]
fn a_damaged_file_list_segment_does_not_lock_you_out() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    tree(&src);
    let vpath = tmp.path().join("v");
    let (mut v, _) = Vault::create(&vpath, PW, fast()).unwrap();
    v.create_folder(None, "kept").unwrap();
    v.import(&[src.join("proj")], None, false, &mut |_| {})
        .unwrap();
    let mut segs = all_files(&vpath.join(MANIFEST_DIR));
    segs.sort();
    assert!(segs.len() >= 2);
    drop(v);
    // Break the last segment (the import), as a bad sync might.
    fs::write(segs.last().unwrap(), b"garbage garbage garbage").unwrap();
    let mut v = Vault::unlock(&vpath, &Unlock::Password(PW)).unwrap();
    assert_eq!(v.manifest().damaged.len(), 1);
    assert!(v.manifest().children(None).iter().any(|e| e.name == "kept"));
    let r = v.check(&mut |_| {}).unwrap();
    assert_eq!(r.damaged_segments.len(), 1);
    // The import's blobs now look unlisted; old enough, they are orphans.
    for b in all_files(&vpath.join(OBJECTS_DIR)) {
        age(&b, 3 * 24 * 3600);
    }
    let r = v.check(&mut |_| {}).unwrap();
    assert!(!r.orphans.is_empty());
    let fixed = v.repair(false).unwrap();
    assert_eq!(fixed.segments_set_aside, 1);
    assert_eq!(all_files(&vpath.join(check::DAMAGED_DIR)).len(), 1);
    drop(v);
    let mut v = Vault::unlock(&vpath, &Unlock::Password(PW)).unwrap();
    assert!(v.manifest().damaged.is_empty());
    assert!(v.check(&mut |_| {}).unwrap().is_clean());

    // If nothing at all can be read, unlocking fails instead of showing an
    // empty vault that could then overwrite the real list.
    drop(v);
    for s in all_files(&vpath.join(MANIFEST_DIR)) {
        fs::write(s, b"garbage garbage garbage").unwrap();
    }
    assert!(Vault::unlock(&vpath, &Unlock::Password(PW)).is_err());
}

#[test]
fn lost_items_move_to_the_top() {
    let tmp = tempfile::tempdir().unwrap();
    let vpath = tmp.path().join("v");
    let (mut v, _) = Vault::create(&vpath, PW, fast()).unwrap();
    let a = v.create_folder(None, "a").unwrap();
    let b = v.create_folder(Some(a), "b").unwrap();
    // A sync that brought the child but lost its parent.
    v.manifest.commit(vec![Record::Remove { id: a }]).unwrap();
    let r = v.check(&mut |_| {}).unwrap();
    assert_eq!(r.lost.len(), 1);
    assert_eq!(v.repair(false).unwrap().moved_to_top, 1);
    assert_eq!(v.manifest().get(&b).unwrap().parent, None);
}

#[test]
fn password_cost_can_change_with_the_password() {
    let tmp = tempfile::tempdir().unwrap();
    let vpath = tmp.path().join("v");
    let (mut v, _) = Vault::create(&vpath, PW, fast()).unwrap();
    assert_eq!(v.password_cost(), Some(fast()));
    let light = Argon2Cost {
        memory_kib: 64 * 1024,
        iterations: 2,
        parallelism: 1,
    };
    assert!(v
        .set_password_cost(b"not the password at all", light)
        .is_err());
    v.set_password_cost(PW, light).unwrap();
    assert_eq!(v.password_cost(), Some(light));
    drop(v);
    let v = Vault::unlock(&vpath, &Unlock::Password(PW)).unwrap();
    assert_eq!(v.password_cost(), Some(light));
}

#[test]
fn rekey_replaces_every_key_and_keeps_every_file() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    tree(&src);
    let vpath = tmp.path().join("v");
    let (mut v, old_phrase) = Vault::create(&vpath, PW, fast()).unwrap();
    v.import(&[src.join("proj")], None, false, &mut |_| {})
        .unwrap();
    let old_id = v.vault_id().to_vec();
    let old_blobs = all_files(&vpath.join(OBJECTS_DIR));
    let device = b"phone".to_vec();
    v.enroll_device(&device, "Phone").unwrap();

    assert!(v.rekey(b"wrong password!!", fast(), &mut |_| {}).is_err());
    assert_eq!(
        v.vault_id(),
        &old_id[..],
        "a refused re-key changes nothing"
    );

    let mut last = Progress::default();
    let phrase = v.rekey(PW, fast(), &mut |p| last = p.clone()).unwrap();
    assert_eq!(last.files_done, last.files_total);
    assert_ne!(v.vault_id(), &old_id[..]);
    assert_ne!(phrase.as_str(), old_phrase.as_str());
    assert!(v.check(&mut |_| {}).unwrap().is_clean());
    // Every blob is new; nothing of the old vault is left in the folder.
    assert!(old_blobs.iter().all(|b| !b.exists()));
    let siblings: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(siblings.len(), 2, "{siblings:?}");
    // Fingerprint slots are not carried over.
    assert!(v.device_expires(&device).is_none());

    let out = tmp.path().join("out");
    let ids: Vec<Id> = v.manifest().children(None).iter().map(|e| e.id).collect();
    v.export(&ids, &out, &mut |_| {}).unwrap();
    same_tree(&src.join("proj"), &out.join("proj"));
    drop(v);

    // The old phrase no longer opens this copy; the new one does.
    let old_key = recovery::from_phrase(&old_phrase).unwrap();
    assert!(Vault::unlock(&vpath, &Unlock::Recovery(&old_key)).is_err());
    let new_key = recovery::from_phrase(&phrase).unwrap();
    Vault::unlock(&vpath, &Unlock::Recovery(&new_key)).unwrap();
    Vault::unlock(&vpath, &Unlock::Password(PW)).unwrap();
}

#[test]
fn an_interrupted_rekey_swap_puts_the_old_vault_back() {
    let tmp = tempfile::tempdir().unwrap();
    let vpath = tmp.path().join("v");
    let (mut v, _) = Vault::create(&vpath, PW, fast()).unwrap();
    v.create_folder(None, "kept").unwrap();
    drop(v);
    // The moment between moving the old vault aside and the new one in.
    fs::rename(&vpath, tmp.path().join(".v.old-12345678")).unwrap();
    let v = Vault::unlock(&vpath, &Unlock::Password(PW)).unwrap();
    assert!(v.manifest().children(None).iter().any(|e| e.name == "kept"));
}

#[test]
fn edited_contents_replace_the_old_blob() {
    let tmp = tempfile::tempdir().unwrap();
    let vpath = tmp.path().join("v");
    let (mut v, _) = Vault::create(&vpath, PW, fast()).unwrap();
    let src = tmp.path().join("notes.txt");
    fs::write(&src, b"first version").unwrap();
    v.import(&[src], None, false, &mut |_| {}).unwrap();
    let id = v.manifest().children(None)[0].id;
    let before = all_files(&vpath.join(OBJECTS_DIR));
    v.replace_contents(id, "second, longer version ✓".as_bytes())
        .unwrap();
    let mut out = Vec::new();
    v.read_file(id, &mut out).unwrap();
    assert_eq!(out, "second, longer version ✓".as_bytes());
    let e = v.manifest().get(&id).unwrap();
    assert_eq!(e.size, out.len() as u64);
    assert_eq!(e.name, "notes.txt");
    assert!(before.iter().all(|b| !b.exists()), "old blob removed");
    assert!(v.check(&mut |_| {}).unwrap().is_clean());
    let folder = v.create_folder(None, "dir").unwrap();
    assert!(v.replace_contents(folder, b"x").is_err());
}
