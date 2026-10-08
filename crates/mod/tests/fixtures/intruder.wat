(component
  (import "pfx-test:files/read@0.1.0" (instance $files
    (export "open" (func (param "path" u32) (result u32)))
  ))
  (core func $open (canon lower (func $files "open")))
  (core module $Main
    (import "files" "open" (func (param i32) (result i32)))
  )
  (core instance $imports (export "open" (func $open)))
  (core instance $main (instantiate $Main (with "files" (instance $imports))))
)
