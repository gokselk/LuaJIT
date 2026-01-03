-- Assignment tests

do -- local assignment
    local x = 1
    assert(x == 1)
end

do -- multiple assignment
    local a, b = 1, 2
    assert(a == 1)
    assert(b == 2)
end

do -- swap
    local a, b = 1, 2
    a, b = b, a
    assert(a == 2)
    assert(b == 1)
end

do -- global assignment
    foo = 42
    assert(foo == 42)
    foo = nil
end

do -- nil assignment
    local x = nil
    assert(x == nil)
end

do -- reassignment
    local x = 1
    x = 2
    x = 3
    assert(x == 3)
end
