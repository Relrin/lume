-- Runs one program on the machine of the server whose program reads this, and hands what
-- it does to the client that asked for it. What to run, what the program reads and what
-- it writes are all requests that client answers; nothing is written to a disk.

-- What shows the client that this is the run it asked for: typed, not an argument, which
-- others on the machine could read.
local SECRET = (io.read("*l") or ""):gsub("[^%x]", "")
local CLIENT = "@CLIENT@"
-- How often the client is asked whether it is still there, in seconds.
local BEAT = 2

-- A byte above 127 is sent as the character of the same number. Texts are gone through in
-- short pieces, since replacing in a long one takes the square of its length.
local PIECE = 128
local up, down = {}, {}
for byte = 128, 255 do
  local char = string.char(0xC0 + math.floor(byte / 64), 0x80 + byte % 64)
  up[string.char(byte)] = char
  down[char] = string.char(byte)
end

local function to_text(bytes)
  if not bytes:find("[\128-\255]") then return bytes end
  local pieces = {}
  for at = 1, #bytes, PIECE do
    pieces[#pieces + 1] = (bytes:sub(at, at + PIECE - 1):gsub("[\128-\255]", up))
  end
  return table.concat(pieces)
end

local function to_bytes(pieces)
  local bytes = {}
  for nth, piece in ipairs(pieces) do
    bytes[nth] = (piece:gsub("[\194\195][\128-\191]", down))
  end
  return table.concat(bytes)
end

-- A client that is gone, or wants the run no more, ends it: the program goes with this.
local function ask(action, args)
  args.secret = SECRET
  local answer, why = rex.call("client.perform", {
    client_id = CLIENT,
    action = "lume.run." .. action,
    args = args,
    timeout_ms = 30000,
  })
  if not answer then
    print(why)
    rex.stop(0)
  end
  return answer.result or {}
end

local job = ask("job", {})

local function send(to, bytes)
  for at = 1, #bytes, job.part do
    local answer = ask("out", { to = to, data = to_text(bytes:sub(at, at + job.part - 1)) })
    -- Nobody has read what was sent so far: the program waits while this does.
    local pause = 0.05
    while answer.hold do
      rex.sleep(pause)
      pause = math.min(pause * 2, 1)
      answer = ask("room", { to = to })
    end
  end
end

-- What is past the limit is read and dropped, so the program ends as it would have.
local function keep(kept)
  return function(bytes)
    local room = job.max - kept.size
    if room > 0 then
      bytes = bytes:sub(1, room)
      kept.size = kept.size + #bytes
      kept[#kept + 1] = bytes
    end
  end
end

local out, err = { size = 0 }, { size = 0 }
local folder = job.folder or os.getenv("HOME") or os.getenv("USERPROFILE")
local options = { cwd = folder, env = job.env, timeout = job.timeout, text = false }
if job.input then
  local parts = {}
  while true do
    local part = ask("in", {})
    if not part.data then break end
    parts[#parts + 1] = to_bytes(part.data)
  end
  options.stdin = table.concat(parts)
end
if job.stream then
  options.stdout = function(bytes) send("out", bytes) end
  options.stderr = function(bytes) send("err", bytes) end
else
  options.stdout = keep(out)
  options.stderr = keep(err)
end

local program, why = rex.exec.start(job.argv, options)
if not program then
  -- The folder is at fault where nothing runs in it.
  local runs = rex.exec({ "rex", "--version" }, { cwd = folder, stdout = false, stderr = false })
  ask("done", { failed = why, folder = not runs })
  rex.stop(0)
end
if job.stream then ask("began", {}) end
local ended
repeat
  ended = program:wait(BEAT)
  if not ended then ask("alive", {}) end
until ended
if not job.stream then
  send("out", table.concat(out))
  send("err", table.concat(err))
end
ask("done", { code = ended.code, signal = ended.signal, timed_out = ended.timed_out })
rex.stop(0)
