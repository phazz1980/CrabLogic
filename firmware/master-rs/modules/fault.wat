;; Test module for checking the safe state by hand (ABI v4).
;; Every step it writes a level to a pin and reports it; an input makes it fail on purpose.
;;   param fault 0 i <pin + (on_fault << 9)>   pin (0 = pin 2); on_fault 0 low, 1 high, 2 hold,
;;                                             e.g. 514 = pin 2 with on_fault = 1
;;   param fault 1 i 2                         level 0 (any other value, including 0: level 1)
;;   in fault 0 i 1                            trap (`unreachable`)
;;   in fault 1 i 1                            endless loop (the master ends the step: "too long")
;; Expected: the master puts the pin into its on_fault state and restarts the module after a pause.
(module
  (@custom "crab.mem" "\00\00\00\00")
  (import "crab" "input_i" (func $input_i (param i32) (result i32)))
  (import "crab" "param_i" (func $param_i (param i32) (result i32)))
  (import "crab" "output_i" (func $output_i (param i32 i32) (result i32)))
  (import "crab" "gpio_write" (func $gpio_write (param i32 i32)))
  (import "crab" "yield" (func $yield (result i32)))
  ;; Work counter as in generated modules: one unit per loop pass, `yield` refills it.
  (global $fuel (mut i32) (i32.const 10000))
  (func (export "abi_version") (result i32) i32.const 4)
  (func (export "step") (param $dt i32)
    (local $pin i32)
    (local $level i32)
    (local.set $pin (call $param_i (i32.const 0)))
    (if (i32.eqz (local.get $pin)) (then (local.set $pin (i32.const 2))))
    (local.set $level (i32.ne (call $param_i (i32.const 1)) (i32.const 2)))
    (call $gpio_write (local.get $pin) (local.get $level))
    (drop (call $output_i (i32.const 0) (local.get $level)))
    (if (i32.ne (call $input_i (i32.const 0)) (i32.const 0))
      (then unreachable))
    (if (i32.ne (call $input_i (i32.const 1)) (i32.const 0))
      (then
        (loop $forever
          (global.set $fuel (i32.sub (global.get $fuel) (i32.const 1)))
          (if (i32.lt_s (global.get $fuel) (i32.const 1))
            (then (global.set $fuel (call $yield))))
          (br $forever))))))
