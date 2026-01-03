-- Operator tests

do -- arithmetic
    assert(1 + 2 == 3)
    assert(5 - 3 == 2)
    assert(3 * 4 == 12)
    assert(10 / 2 == 5)
    assert(2 ^ 3 == 8)
    assert(7 % 3 == 1)
end

do -- unary minus
    assert(-5 == 0 - 5)
    local x = 10
    assert(-x == -10)
end

do -- comparison
    assert(1 < 2)
    assert(2 > 1)
    assert(1 <= 1)
    assert(1 <= 2)
    assert(2 >= 2)
    assert(2 >= 1)
    assert(1 == 1)
    assert(1 ~= 2)
end

do -- logical
    assert(true and true)
    assert(not (true and false))
    assert(true or false)
    assert(not (false or false))
    assert(not false)
    assert(not not true)
end

do -- short circuit
    local x = false and error("should not evaluate")
    assert(x == false)

    local y = true or error("should not evaluate")
    assert(y == true)
end

do -- string concatenation
    assert("hello" .. " " .. "world" == "hello world")
    assert("x" .. 1 == "x1")
    assert(1 .. 2 == "12")
end

do -- length
    assert(#"hello" == 5)
    assert(#{1,2,3} == 3)
end
