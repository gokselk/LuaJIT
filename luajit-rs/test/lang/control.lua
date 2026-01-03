-- Control flow tests

do -- if-then
    local x = 0
    if true then
        x = 1
    end
    assert(x == 1)
end

do -- if-then-else
    local x
    if false then
        x = 1
    else
        x = 2
    end
    assert(x == 2)
end

do -- if-elseif-else
    local function test(n)
        if n < 0 then
            return "negative"
        elseif n == 0 then
            return "zero"
        else
            return "positive"
        end
    end
    assert(test(-1) == "negative")
    assert(test(0) == "zero")
    assert(test(1) == "positive")
end

do -- while loop
    local sum = 0
    local i = 1
    while i <= 10 do
        sum = sum + i
        i = i + 1
    end
    assert(sum == 55)
end

do -- repeat-until
    local sum = 0
    local i = 1
    repeat
        sum = sum + i
        i = i + 1
    until i > 10
    assert(sum == 55)
end

do -- numeric for
    local sum = 0
    for i = 1, 10 do
        sum = sum + i
    end
    assert(sum == 55)
end

do -- numeric for with step
    local sum = 0
    for i = 0, 10, 2 do
        sum = sum + i
    end
    assert(sum == 30) -- 0+2+4+6+8+10
end

do -- numeric for negative step
    local result = {}
    for i = 5, 1, -1 do
        table.insert(result, i)
    end
    assert(result[1] == 5)
    assert(result[5] == 1)
end

do -- break
    local last = 0
    for i = 1, 100 do
        last = i
        if i == 5 then break end
    end
    assert(last == 5)
end
