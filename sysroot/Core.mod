module Core;
: _CONTRACT_FAIL ( -- i64 ) 20 ;
: _SUBTYPE_FAIL ( -- i64 ) 21 ;
: _ASSERT_FAIL ( -- i64 ) 22 ;
: _UNREACHABLE ( -- i64 ) 23 ;
: _STACK_OVERFLOW ( -- i64 ) 10 ;
export {
  _CONTRACT_FAIL,
  _SUBTYPE_FAIL,
  _ASSERT_FAIL,
  _UNREACHABLE,
  _STACK_OVERFLOW,
};
end;
