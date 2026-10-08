(component
  (core module $Main
    (func $spin
      (loop $again
        (br $again)))
    (start $spin)
  )
  (core instance $main (instantiate $Main))
)
