# Builds the speech samples in test-files/audio/ (Windows; needs ffmpeg on PATH).
# A synthetic "lecture announcement" with real dates, in the formats people
# actually have: a WAV, a phone voice memo (M4A/AAC), a screen recording
# (MP4 video with AAC audio), an MP3, and a WebM/Opus file that MyFile's
# built-in decoder can't read (so it exercises the ffmpeg fallback).
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Speech

$out = Join-Path $PSScriptRoot "audio"
New-Item -ItemType Directory -Force $out | Out-Null
$wav = Join-Path $out "bio-200-announcements.wav"

$p = New-Object System.Speech.Synthesis.PromptBuilder
$p.AppendText("Good morning everyone, and welcome back to Biology 200. A few announcements before we start.")
$p.AppendText("The midterm exam has moved. It will now be on Thursday, November 19th, in this room.")
$p.AppendText("Lab reports are due every Friday by 5 PM, and late lab reports are not accepted.")
$p.AppendBreak([TimeSpan]::FromSeconds(3))
$p.AppendText("Your group project proposal is due on December 1st. Office hours this week are Tuesday from 2 to 4 in Hall 210.")
$p.AppendText("Okay, let's get started with cell membranes.")

$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
# 44.1 kHz stereo, so MyFile has to downmix and resample.
$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(44100, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Stereo)
$s.SetOutputToWaveFile($wav, $fmt)
$s.Speak($p)
$s.Dispose()

Push-Location $out
ffmpeg -y -v error -i bio-200-announcements.wav -c:a aac -b:a 96k bio-200-voice-memo.m4a
ffmpeg -y -v error -f lavfi -i "testsrc=size=640x360:rate=15" -i bio-200-announcements.wav -shortest -c:v libx264 -pix_fmt yuv420p -c:a aac -b:a 96k bio-200-lecture-recording.mp4
ffmpeg -y -v error -i bio-200-announcements.wav -c:a libmp3lame -b:a 64k bio-200-announcements.mp3
ffmpeg -y -v error -i bio-200-announcements.wav -c:a libopus -b:a 32k bio-200-announcements.webm
Pop-Location
Get-ChildItem $out | Select-Object Name, Length
