module platform/time;
: platform.time.now_us ( -- i64 )
  # RP2350: implemented by the metal runtime asm (TIMER0), metal.trust
;
: platform.time.reboot ( -- )
  # RP2350: implemented by the metal runtime asm, metal.trust
;
export { platform.time.now_us platform.time.reboot }
end;
