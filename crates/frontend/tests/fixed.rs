// Miri tests for FixedVec, IterMut, IntoIter — exercises all unsafe code paths.
// Run: cargo +nightly miri test -p frontend --test fixed

use frontend::fixed::FixedVec;

#[test]
fn new_vec_is_empty() {
    let v: FixedVec<u64, 8> = FixedVec::new();
    assert_eq!(v.len(), 0);
    assert!(v.is_empty());
}

#[test]
fn push_and_get_roundtrip() {
    let mut v: FixedVec<u64, 4> = FixedVec::new();
    for i in 0..4 {
        assert!(v.push(i).is_ok());
    }
    assert!(v.push(4).is_err()); // overflow
    for i in 0..4 {
        assert_eq!(*v.get(i).unwrap(), i as u64);
    }
    assert!(v.get(4).is_none());
}

#[test]
fn get_mut_allows_mutation() {
    let mut v: FixedVec<u64, 4> = FixedVec::new();
    v.push(10).unwrap();
    *v.get_mut(0).unwrap() = 20;
    assert_eq!(*v.get(0).unwrap(), 20);
}

#[test]
fn iter_empty() {
    let v: FixedVec<u64, 8> = FixedVec::new();
    assert_eq!(v.iter().count(), 0);
}

#[test]
fn iter_yields_all_elements() {
    let mut v: FixedVec<u64, 8> = FixedVec::new();
    v.push(1).unwrap();
    v.push(2).unwrap();
    v.push(3).unwrap();
    let mut count = 0;
    let mut sum = 0;
    for x in v.iter() {
        count += 1;
        sum += *x;
    }
    assert_eq!(count, 3);
    assert_eq!(sum, 6);
}

#[test]
fn iter_mut_yields_mutable_refs() {
    let mut v: FixedVec<u64, 8> = FixedVec::new();
    v.push(10).unwrap();
    v.push(20).unwrap();
    v.push(30).unwrap();
    for x in v.iter_mut() {
        *x *= 2;
    }
    assert_eq!(*v.get(0).unwrap(), 20);
    assert_eq!(*v.get(1).unwrap(), 40);
    assert_eq!(*v.get(2).unwrap(), 60);
}

#[test]
fn iter_mut_empty() {
    let mut v: FixedVec<u64, 4> = FixedVec::new();
    assert!(v.iter_mut().next().is_none());
}

// ----- IterMut stacked borrows check -----
// This is the key Miri test: two &mut T from iter_mut must not alias.
#[test]
fn iter_mut_aliasing() {
    let mut v: FixedVec<u64, 8> = FixedVec::new();
    v.push(1).unwrap();
    v.push(2).unwrap();
    let mut iter = v.iter_mut();
    let a = iter.next().unwrap();
    let b = iter.next().unwrap();
    *a += 10;
    *b += 20;
    assert_eq!(*a, 11);
    assert_eq!(*b, 22);
}

// ----- IntoIter consumption -----
#[test]
fn into_iter_consumes_all() {
    let mut v: FixedVec<u64, 4> = FixedVec::new();
    v.push(1).unwrap();
    v.push(2).unwrap();
    let mut sum = 0u64;
    for x in v.into_iter() {
        sum += x;
    }
    assert_eq!(sum, 3);
}

#[test]
fn into_iter_empty() {
    let v: FixedVec<u64, 8> = FixedVec::new();
    assert_eq!(v.into_iter().count(), 0);
}

// ----- IntoIter partial drop (Miri must not detect double-free) -----
#[test]
fn into_iter_partial_drop() {
    let mut v: FixedVec<u64, 4> = FixedVec::new();
    v.push(1).unwrap();
    v.push(2).unwrap();
    v.push(3).unwrap();
    let mut iter = v.into_iter();
    assert_eq!(iter.next(), Some(1));
    // Drop iter here — remaining elements (2, 3) must be dropped exactly once.
    drop(iter);
}

// ----- Drop correctness with heap-allocated content (Miri leak check) -----
#[test]
fn drop_vec_with_strings() {
    let mut v: FixedVec<String, 4> = FixedVec::new();
    v.push("hello".into()).unwrap();
    v.push("world".into()).unwrap();
    v.push("foo".into()).unwrap();
}

#[test]
fn into_iter_with_strings() {
    let mut v: FixedVec<String, 4> = FixedVec::new();
    v.push("a".into()).unwrap();
    v.push("b".into()).unwrap();
    for _ in v.into_iter() {}
}

// ----- IntoIter partial drop with heap content -----
#[test]
fn into_iter_partial_drop_strings() {
    let mut v: FixedVec<String, 4> = FixedVec::new();
    v.push("x".into()).unwrap();
    v.push("y".into()).unwrap();
    v.push("z".into()).unwrap();
    let mut iter = v.into_iter();
    assert_eq!(iter.next(), Some("x".into()));
    drop(iter); // "y" and "z" must be dropped without double-free
}

// ----- Push to capacity then overflow -----
#[test]
fn push_to_capacity_then_overflow() {
    let mut v: FixedVec<i32, 3> = FixedVec::new();
    assert!(v.push(1).is_ok());
    assert!(v.push(2).is_ok());
    assert!(v.push(3).is_ok());
    assert!(v.push(4).is_err());
    assert_eq!(v.len(), 3);
}

// ----- len/is_empty after push/pop-like operations -----
#[test]
fn len_tracks_pushes() {
    let mut v: FixedVec<u64, 8> = FixedVec::new();
    assert_eq!(v.len(), 0);
    v.push(10).unwrap();
    assert_eq!(v.len(), 1);
    v.push(20).unwrap();
    assert_eq!(v.len(), 2);
}
