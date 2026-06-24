module platform/time;
: platform.time.now_us ( -- i64 )
  # bare-metal: monotonic QEMU timestamp from runtime assembly
;
: platform.time.reboot ( -- )
  # bare-metal: halt-only stub backed by runtime assembly
;
export { platform.time.now_us platform.time.reboot };
end;
