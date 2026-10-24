"""A simulation of the bank account, written from the contract and the interface only (blind): it never reads the
program's code.

Contract `account`:
  C1  a deposit adds its amount to the balance
  C2  a withdrawal of more than the balance is refused and leaves the balance as it was

Protocol chidev-sim 2:  sim.py <case> --out <dir> -- <program argv...>
The case is `seed-<n>` or `regress-<name>`. The script draws everything from the seed (nothing from the clock or
the environment), writes `schedule` and `trace` into <dir>, and prints the report. A clause is named
`<contract>:<label>` (`account:C2`): labels are unique only within a contract.
"""
import json
import random
import subprocess
import sys
import time

CLOCK = False  # a script that reads the clock is not repeatable
CONTRACT = "account"


def schedule_of(case):
    if case.startswith("seed-"):
        rng = random.Random(int(case[len("seed-"):]))
        steps = []
        for _ in range(12):
            r = rng.random()
            if r < 0.4:
                steps.append(["deposit", rng.randint(1, 50)])
            elif r < 0.8:
                steps.append(["withdraw", rng.randint(1, 60)])
            else:
                steps.append(["balance", 0])
        return steps
    if case == "regress-overdraw":
        return [["withdraw", 5], ["balance", 0]]
    raise SystemExit("unknown case " + case)


def main():
    case, flag, out, dash = sys.argv[1:5]
    assert flag == "--out" and dash == "--"
    program = sys.argv[5:]
    steps = schedule_of(case)
    with open(out + "/schedule", "w") as f:
        f.write(json.dumps(steps))
        if CLOCK:
            f.write(str(time.time_ns()))
    stdin = "".join("%s %d\n" % (cmd, n) if cmd != "balance" else "balance\n" for cmd, n in steps)
    ran = subprocess.run(program, input=stdin, capture_output=True, text=True, timeout=20)
    with open(out + "/trace", "w") as f:
        f.write(ran.stdout)
    answers = ran.stdout.splitlines()

    findings = []
    overdrawn = False
    model = 0
    for k, ((cmd, n), answer) in enumerate(zip(steps, answers), start=1):
        if cmd == "deposit":
            model += n
            expected = "ok %d" % model
            label = CONTRACT + ":C1"
        elif cmd == "withdraw":
            if n > model:
                overdrawn = True
                expected = "refused %d" % model
            else:
                model -= n
                expected = "ok %d" % model
            label = CONTRACT + ":C2"
        else:
            expected = "balance %d" % model
            label = None
        if answer != expected:
            if label:
                findings.append("FINDING %s step %d %s: %s %d gave `%s`, the clause wants `%s`" % (case, k, label, cmd, n, answer, expected))
            break
    if len(answers) < len(steps) and not findings:
        findings.append("FINDING %s step %d %s:C1: the program stopped answering" % (case, len(answers) + 1, CONTRACT))

    for line in findings:
        print(line)
    if not overdrawn:
        print("UNREACHED %s:C2: no withdrawal of more than the balance" % CONTRACT)
    print("sim: findings=%d" % len(findings))
    sys.exit(1 if findings else 0)


main()
