-- Table tests

do -- array construction
    local t = {1, 2, 3}
    assert(t[1] == 1)
    assert(t[2] == 2)
    assert(t[3] == 3)
    assert(#t == 3)
end

do -- hash construction
    local t = {a = 1, b = 2}
    assert(t.a == 1)
    assert(t["b"] == 2)
end

do -- mixed construction
    local t = {1, 2, a = "x", 3}
    assert(t[1] == 1)
    assert(t[2] == 2)
    assert(t[3] == 3)
    assert(t.a == "x")
end

do -- assignment
    local t = {}
    t[1] = "a"
    t.foo = "b"
    t["bar"] = "c"
    assert(t[1] == "a")
    assert(t.foo == "b")
    assert(t.bar == "c")
end

do -- nested tables
    local t = {inner = {x = 1}}
    assert(t.inner.x == 1)
    t.inner.y = 2
    assert(t.inner.y == 2)
end

do -- table.insert
    local t = {1, 2, 3}
    table.insert(t, 4)
    assert(t[4] == 4)
    assert(#t == 4)
end

do -- table.remove
    local t = {1, 2, 3}
    local v = table.remove(t)
    assert(v == 3)
    assert(#t == 2)
end

do -- table.concat
    local t = {"a", "b", "c"}
    assert(table.concat(t) == "abc")
    assert(table.concat(t, ",") == "a,b,c")
end

do -- ipairs
    local t = {10, 20, 30}
    local sum = 0
    for i, v in ipairs(t) do
        sum = sum + v
    end
    assert(sum == 60)
end

do -- pairs
    local t = {a = 1, b = 2}
    local sum = 0
    for k, v in pairs(t) do
        sum = sum + v
    end
    assert(sum == 3)
end
