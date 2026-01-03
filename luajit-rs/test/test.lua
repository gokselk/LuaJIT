-- LuaJIT-RS Test Suite Runner
-- Usage: luajit-rs test/test.lua [pattern]

local passed = 0
local failed = 0
local skipped = 0
local errors = {}

-- ANSI colors
local function green(s) return "\27[32m" .. s .. "\27[0m" end
local function red(s) return "\27[31m" .. s .. "\27[0m" end
local function yellow(s) return "\27[33m" .. s .. "\27[0m" end

-- Get test files from a directory
local function get_tests(dir)
    local tests = {}
    -- Read index file if it exists
    local f = io.open(dir .. "/index", "r")
    if f then
        for line in f:lines() do
            line = line:match("^%s*(.-)%s*$") -- trim
            if line ~= "" and not line:match("^#") then
                table.insert(tests, dir .. "/" .. line)
            end
        end
        f:close()
    end
    return tests
end

-- Run a single test file
local function run_test(file)
    local f, err = loadfile(file)
    if not f then
        return false, "load error: " .. tostring(err)
    end

    local ok, err = pcall(f)
    if not ok then
        return false, tostring(err)
    end

    return true
end

-- Main test runner
local function run_tests(pattern)
    pattern = pattern or ""

    print("LuaJIT-RS Test Suite")
    print("====================")
    print("")

    -- Test directories to scan
    local dirs = {"test/lang", "test/lib", "test/misc"}

    for _, dir in ipairs(dirs) do
        local tests = get_tests(dir)

        if #tests > 0 then
            print("Running " .. dir .. " (" .. #tests .. " tests)")

            for _, file in ipairs(tests) do
                -- Check pattern filter
                if pattern == "" or file:match(pattern) then
                    io.write("  " .. file .. " ... ")
                    io.flush()

                    local ok, err = run_test(file)

                    if ok then
                        print(green("PASS"))
                        passed = passed + 1
                    else
                        print(red("FAIL"))
                        failed = failed + 1
                        table.insert(errors, {file = file, err = err})
                    end
                end
            end
            print("")
        end
    end

    -- Summary
    print("====================")
    print("Results:")
    print("  " .. green(passed .. " passed"))
    if failed > 0 then
        print("  " .. red(failed .. " failed"))
    end
    if skipped > 0 then
        print("  " .. yellow(skipped .. " skipped"))
    end
    print("")

    -- Show errors
    if #errors > 0 then
        print("Failures:")
        for _, e in ipairs(errors) do
            print("  " .. e.file .. ":")
            print("    " .. e.err)
        end
        print("")
    end

    -- Exit with appropriate code
    if failed > 0 then
        os.exit(1)
    end
end

-- Run
local pattern = arg and arg[1] or ""
run_tests(pattern)
