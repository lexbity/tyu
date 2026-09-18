module ResourceLockHosted;
resource Counter : i64 = 0 ceiling 1;

: bump ( -- )
  Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ]
;

export { bump };
end;