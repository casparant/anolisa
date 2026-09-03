//! Chain state tests.

use crate::Chain;

use super::{admit_and_extend, clean_body};

#[test]
fn empty_chain_has_no_tip() {
    let chain = Chain::new();
    let tip = chain.tip();
    assert_eq!(tip.sequence, 0);
    assert!(tip.id.is_none());
    assert!(tip.digest.is_none());
}

#[test]
fn extend_advances_tip_and_sequence() {
    let mut chain = Chain::new();
    let genesis = admit_and_extend(&mut chain, clean_body());

    let tip = chain.tip();
    assert_eq!(tip.sequence, 0);
    assert_eq!(tip.id, Some(&genesis.header.id));
    assert_eq!(tip.digest, Some(&genesis.record_digest));
}

#[test]
fn two_records_produce_a_continuous_chain() {
    let mut chain = Chain::new();
    let genesis = admit_and_extend(&mut chain, clean_body());
    let second = admit_and_extend(&mut chain, clean_body());

    let tip = chain.tip();
    assert_eq!(tip.sequence, 1);
    assert_eq!(tip.id, Some(&second.header.id));
    assert_eq!(tip.digest, Some(&second.record_digest));

    // The second record's parent link commits to the genesis record.
    let parent = second
        .header
        .parent
        .as_ref()
        .expect("non-genesis has a parent");
    assert_eq!(parent.id, genesis.header.id);
    assert_eq!(parent.digest, genesis.record_digest);
}
