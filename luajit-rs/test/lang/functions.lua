-- Function tests

do -- basic function
    local function add(a, b)
        return a + b
    end
    assert(add(2, 3) == 5)
end

do -- multiple returns
    local function swap(a, b)
        return b, a
    end
    local x, y = swap(1, 2)
    assert(x == 2)
    assert(y == 1)
end

do -- recursion
    local function factorial(n)
        if n <= 1 then return 1 end
        return n * factorial(n - 1)
    end
    assert(factorial(0) == 1)
    assert(factorial(1) == 1)
    assert(factorial(5) == 120)
end

do -- fibonacci
    local function fib(n)
        if n <= 1 then return n end
        return fib(n - 1) + fib(n - 2)
    end
    assert(fib(0) == 0)
    assert(fib(1) == 1)
    assert(fib(10) == 55)
end

do -- tail recursion
    local function sum(n, acc)
        acc = acc or 0
        if n == 0 then return acc end
        return sum(n - 1, acc + n)
    end
    assert(sum(10) == 55)
end

do -- varargs
    local function count(...)
        return select("#", ...)
    end
    assert(count() == 0)
    assert(count(1) == 1)
    assert(count(1, 2, 3) == 3)
end

do -- function as value
    local function apply(f, x)
        return f(x)
    end
    local function double(x)
        return x * 2
    end
    assert(apply(double, 5) == 10)
end

do -- anonymous function
    local f = function(x) return x * x end
    assert(f(4) == 16)
end

do -- nested function calls
    local function a() return 1 end
    local function b() return a() + 1 end
    local function c() return b() + 1 end
    assert(c() == 3)
end
