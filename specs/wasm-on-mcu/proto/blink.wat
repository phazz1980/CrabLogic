;; Prototype module: toggles a pin every 500 ms (ABI v3 subset: gpio_write, millis)
(module
  ;; No memory: the master needs this section to start the module (`specs/mcu-memory`).
  (@custom "crab.mem" "\00\00\00\00")
  (import "crab" "gpio_write" (func $gpio_write (param i32 i32)))
  (import "crab" "millis" (func $millis (result i32)))
  (global $last (mut i32) (i32.const 0))
  (global $state (mut i32) (i32.const 0))
  (func (export "abi_version") (result i32) i32.const 4)
  (func (export "step") (param $dt i32)
    (if (i32.ge_u (i32.sub (call $millis) (global.get $last)) (i32.const 500))
      (then
        (global.set $last (call $millis))
        (global.set $state (i32.eqz (global.get $state)))
        (call $gpio_write (i32.const 2) (global.get $state))))))
