(component
  (import "pfx-test:play/game@0.1.0" (instance $game
    (export "seed" (func (result u32)))
  ))
  (core func $seed (canon lower (func $game "seed")))
  (core module $Main
    (import "game" "seed" (func (result i32)))
  )
  (core instance $imports (export "seed" (func $seed)))
  (core instance $main (instantiate $Main (with "game" (instance $imports))))
)
