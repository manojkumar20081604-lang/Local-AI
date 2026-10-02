#!/usr/bin/env python3
"""Quantize merged model to GGUF for LM Studio / Ollama / llama.cpp.

Order: Unsloth GGUF export -> llama.cpp `llama-quantize` CLI -> manual instructions.
Also writes Modelfile (Ollama) + README hint (LM Studio) next to output.

Usage: python3 finetune/quantize.py --model ./outputs/merged --out ./outputs/gguf --quant q4_k_m
"""
import argparse
import shutil
import subprocess
import sys
from pathlib import Path

QUANTS = {"q4_k_m", "q4_k_s", "q5_k_m", "q5_k_s", "q8_0", "q4_0", "q6_k"}


def run(cmd, **kw):
    print(f"$ {' '.join(map(str, cmd))}")
    return subprocess.run(cmd, **kw)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--quant", default="q4_k_m")
    args = p.parse_args()

    if args.quant not in QUANTS:
        print(f"[warn] unusual quant {args.quant} — expected one of {sorted(QUANTS)}")

    model = Path(args.model)
    if not model.exists():
        print(f"Model not found: {model}", file=sys.stderr)
        sys.exit(1)

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    gguf_path = out_dir / f"model-{args.quant}.gguf"

    # 1) Unsloth GGUF export (best: direct from merged HF dir)
    try:
        from unsloth import FastLanguageModel
        print(f"[unsloth] Exporting {model} -> {gguf_path} ({args.quant})")
        m, tokenizer = FastLanguageModel.from_pretrained(
            model_name=str(model), max_seq_length=4096, dtype=None, load_in_4bit=False)
        m.save_pretrained_gguf(str(out_dir), tokenizer, quantization_method=args.quant)
        print(f"Saved GGUF to {out_dir}")
    except ImportError:
        print("[info] unsloth not installed — trying llama.cpp CLI ...")
    except Exception as e:
        print(f"[warn] Unsloth GGUF export failed: {e} — trying llama.cpp CLI ...")
    else:
        # Unsloth succeeded if any .gguf appeared
        if list(out_dir.glob("*.gguf")):
            gguf_path = sorted(out_dir.glob("*.gguf"))[0]
            write_helpers(gguf_path, out_dir)
            return

    # 2) llama.cpp CLI fallback (convert + quantize)
    quantize_bin = shutil.which("llama-quantize")
    convert_script = shutil.which("convert_hf_to_gguf.py") or shutil.which("convert-hf-to-gguf.py")
    if quantize_bin:
        print(f"[llama.cpp] Found {quantize_bin}")
        # Need F16 GGUF first: try convert script, else instruct
        f16 = out_dir / "model-f16.gguf"
        if convert_script:
            rc = run([sys.executable, convert_script, str(model), "--outfile", str(f16), "--outtype", "f16"])
            if rc.returncode == 0 and f16.exists():
                rc2 = run([quantize_bin, str(f16), str(gguf_path), args.quant])
                if rc2.returncode == 0 and gguf_path.exists():
                    print(f"Saved GGUF to {gguf_path} (llama.cpp)")
                    write_helpers(gguf_path, out_dir)
                    return
        print("[warn] llama.cpp convert/quantize did not produce output.")
    else:
        print("[info] `llama-quantize` not on PATH.")

    print("GGUF export needs one of:")
    print("  pip install unsloth  (then re-run, RTX 5050 recommended)")
    print("  git clone https://github.com/ggerganov/llama.cpp && cmake -B build && cmake --build build")
    print(f"  python convert_hf_to_gguf.py {model} --outfile {out_dir}/model-f16.gguf --outtype f16")
    print(f"  llama-quantize {out_dir}/model-f16.gguf {gguf_path} {args.quant}")
    print("LM Studio can also import the merged HF dir directly (safetensors) without GGUF:")
    print(f"  LM Studio -> My Models -> Add Model -> select {model}")
    sys.exit(1)


def write_helpers(gguf_path: Path, out_dir: Path):
    # Ollama Modelfile
    modelfile = out_dir / "Modelfile"
    if not modelfile.exists():
        modelfile.write_text(
            f'FROM ./{gguf_path.name}\nTEMPLATE """{{{{ if .System }}}}<|im_start|>system\n'
            f'{{{{ .System }}}}<|im_end|>\n{{{{ end }}}}{{{{ if .Prompt }}}}<|im_start|>user\n'
            f'{{{{ .Prompt }}}}<|im_end|>\n{{{{ end }}}}<|im_start|>assistant\n"""\n'
            f'PARAMETER temperature 0.4\nPARAMETER num_ctx 4096\n',
            encoding="utf-8")
        print(f"Wrote {modelfile} (ollama create mymodel -f Modelfile)")
    readme = out_dir / "LOAD_IN_LMSTUDIO.txt"
    if not readme.exists():
        readme.write_text(
            f"Load {gguf_path.name} in LM Studio:\n1. Open LM Studio -> My Models -> Add Model -> Local File\n"
            f"2. Select {gguf_path.resolve()}\n3. Or copy to ~/.lmstudio/models/\n",
            encoding="utf-8")


if __name__ == "__main__":
    main()
