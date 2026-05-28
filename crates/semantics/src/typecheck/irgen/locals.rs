use super::*;

impl<'a, 'r> IrWordGen<'a, 'r> {
    pub(super) fn check_no_scoped_live(&self, stack: &[Value; 256], sp: usize) -> bool {
        check_no_scoped_live(stack, sp)
    }

    pub(super) fn any_scoped_live(&self, stack: &[Value; 256], sp: usize) -> bool {
        if !check_no_scoped_live(stack, sp) {
            return true;
        }
        for i in 0..self.local_len {
            if self.local_live[i] && self.local_scoped[i] != 0 {
                return true;
            }
        }
        false
    }

    pub(super) fn check_no_scoped_live_all(&self, stack: &[Value; 256], sp: usize) -> bool {
        !self.any_scoped_live(stack, sp)
    }

    pub(super) fn enter_scope(&mut self) -> Option<u16> {
        if self.scope_sp >= self.scope_stack.len() {
            return None;
        }
        let id = self.next_scope;
        self.next_scope = self.next_scope.wrapping_add(1);
        self.scope_stack[self.scope_sp] = id;
        self.scope_sp += 1;
        Some(id)
    }

    pub(super) fn leave_scope(&mut self, id: u16) {
        if self.scope_sp == 0 {
            return;
        }
        let top = self.scope_stack[self.scope_sp - 1];
        if top == id {
            self.scope_sp -= 1;
        }
    }

    pub(super) fn stack_has_scope(&self, stack: &[Value; 256], sp: usize, scope: u16) -> bool {
        stack[..sp].iter().any(|v| matches!(v, Value::Scoped { scope: s, .. } if *s == scope))
    }

    pub(super) fn invalidate_scope_locals(&mut self, scope: u16) {
        for i in 0..self.local_len {
            if self.local_scoped[i] == scope {
                self.local_scoped[i] = 0;
                self.local_live[i] = false;
            }
        }
    }

    pub(super) fn local_slot(&self, idx: usize) -> u16 {
        self.sig.in_len as u16 + idx as u16
    }

    pub(super) fn temp_base_slot(&self) -> u16 {
        (self.sig.in_len as u16)
            .wrapping_add(self.local_len as u16)
            .wrapping_add(1)
    }
}
