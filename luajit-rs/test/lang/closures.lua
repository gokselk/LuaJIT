-- Closure and upvalue tests

do -- basic closure
    local x = 10
    local function get_x()
        return x
    end
    assert(get_x() == 10)
end

do -- closure mutation
    local x = 0
    local function inc()
        x = x + 1
        return x
    end
    assert(inc() == 1)
    assert(inc() == 2)
    assert(inc() == 3)
    assert(x == 3)
end

do -- counter factory
    local function make_counter()
        local count = 0
        return function()
            count = count + 1
            return count
        end
    end

    local c1 = make_counter()
    local c2 = make_counter()

    assert(c1() == 1)
    assert(c1() == 2)
    assert(c2() == 1)  -- independent counter
    assert(c1() == 3)
end

do -- nested closures
    local function outer(x)
        return function(y)
            return function(z)
                return x + y + z
            end
        end
    end
    assert(outer(1)(2)(3) == 6)
end

do -- recursive local function
    local function factorial(n)
        if n <= 1 then return 1 end
        return n * factorial(n - 1)
    end
    assert(factorial(5) == 120)
end

do -- mutual recursion
    local even, odd

    function even(n)
        if n == 0 then return true end
        return odd(n - 1)
    end

    function odd(n)
        if n == 0 then return false end
        return even(n - 1)
    end

    assert(even(4) == true)
    assert(even(5) == false)
    assert(odd(3) == true)
    assert(odd(4) == false)
end

do -- upvalue sharing
    local x = 0
    local function inc() x = x + 1 end
    local function get() return x end

    inc()
    inc()
    assert(get() == 2)
end
