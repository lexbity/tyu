use super::*;
use crate::typecheck::value::{PlaceId, PLACE_NONE};

/// A borrowed-place ledger entry tracking one live borrow.
#[derive(Clone, Copy, Debug)]
pub struct PlaceKey {
    pub root: TypeAtom,
    pub full: TypeAtom,
    pub origin: Span,
}

/// Maximum distinct borrowed places tracked per word.
pub const LEDGER_CAP: usize = 64;

/// Scan the stack and local-place arrays for a conflicting borrow.
///
/// Rule (D-4):
///   new is `&!` and ∃ live borrow b (any mutability), root(b) == root(new) → E5021
///   new is `&`  and ∃ live borrow b, b.mutable, root(b) == root(new) → E5021
///   shared/shared coexistence is legal.
///
/// `local_place`/`local_live`/`local_tys` allow scanning borrows shelved in locals.
///
/// Returns `Ok(())` if no conflict, or `Err(TcError::BorrowAlias)` with
/// the current borrow's span and the first conflicting borrow's origin span.
pub fn scan_conflict(
    stack: &[Value; 256],
    sp: usize,
    ledger: &[PlaceKey; LEDGER_CAP],
    ledger_len: u8,
    local_place: &[PlaceId; 64],
    local_live: &[bool; 64],
    local_tys: &[TypeAtom; 64],
    local_len: usize,
    root: TypeAtom,
    mutable: bool,
    new_span: Span,
) -> Result<(), TcError> {
    // Scan stack for conflicting borrows.
    for v in stack[..sp].iter() {
        if let Value::Ptr { place, mutable: m2, .. } = *v {
            if place == PLACE_NONE {
                continue;
            }
            let idx = place.0 as usize;
            if idx >= ledger_len as usize {
                continue;
            }
            if ledger[idx].root == root && (mutable || m2) {
                return Err(TcError::BorrowAlias {
                    span: new_span,
                    first: ledger[idx].origin,
                });
            }
        }
    }
    // Scan live locals for conflicting shelved borrows.
    for i in 0..local_len {
        if !local_live[i] {
            continue;
        }
        let pid = local_place[i];
        if pid == PLACE_NONE {
            continue;
        }
        let local_mutable = local_tys[i] == TypeAtom::PTR_MUT;
        let idx = pid.0 as usize;
        if idx >= ledger_len as usize {
            continue;
        }
        if ledger[idx].root == root && (mutable || local_mutable) {
            return Err(TcError::BorrowAlias {
                span: new_span,
                first: ledger[idx].origin,
            });
        }
    }
    Ok(())
}

/// Mint a new PlaceId for a borrow, interning the (root, full) pair.
///
/// Returns `Err(TcError::BorrowLedgerFull)` if the ledger is full (≥ 64).
pub fn mint_id(
    ledger: &mut [PlaceKey; LEDGER_CAP],
    ledger_len: &mut u8,
    root: TypeAtom,
    full: TypeAtom,
    origin: Span,
) -> Result<PlaceId, TcError> {
    // Check for existing entry (same root+full).
    for (i, entry) in ledger.iter().enumerate().take(*ledger_len as usize) {
        if entry.root == root && entry.full == full {
            return Ok(PlaceId(i as u16));
        }
    }
    if (*ledger_len as usize) >= LEDGER_CAP {
        return Err(TcError::BorrowLedgerFull { span: origin });
    }
    let id = *ledger_len;
    ledger[id as usize] = PlaceKey { root, full, origin };
    *ledger_len += 1;
    Ok(PlaceId(id as u16))
}
