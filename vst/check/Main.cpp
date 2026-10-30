// Loads an installed plugin from VST3/Johnny and puts it through its paces with no audio device:
// every sample rate and block size, random settings, automation mid-block-run, state save and
// load, every parameter's text, the sidechain, and its editor (in a window off screen).

#include <juce_audio_processors/juce_audio_processors.h>
#include <juce_gui_basics/juce_gui_basics.h>
#include <cmath>
#include <cstdio>

static int failures = 0;

static void fail (const juce::String& what)
{
    std::printf ("  FAIL %s\n", what.toRawUTF8());
    ++failures;
}

// Kick-and-saw test signal, left and right a little different.
static void signal (juce::AudioBuffer<float>& b, int start, double sr, int offset = 0)
{
    for (int i = 0; i < b.getNumSamples(); ++i)
    {
        const double t = (start + i) / sr;
        const double k = std::fmod (t, 0.5);
        const double v = 0.6 * std::sin (6.2832 * 60 * k) * std::exp (-k * 9) + 0.1 * (2 * std::fmod (t * 110, 1.0) - 1);
        b.setSample (offset, i, (float) v);
        b.setSample (offset + 1, i, (float) (v * 0.9 + 0.05 * std::sin (6.2832 * 3000 * t)));
    }
}

struct Stats { double peak = 0, rms = 0; int bad = 0; };

// Run for `seconds`; `wiggle` moves a parameter every so often, as automation would.
static Stats run (juce::AudioPluginInstance& p, double sr, int block, double seconds, juce::Random* wiggle = nullptr, bool side = false)
{
    const int channels = juce::jmax (2, p.getTotalNumInputChannels(), p.getTotalNumOutputChannels());
    juce::AudioBuffer<float> buf (channels, block);
    juce::MidiBuffer midi;
    Stats s;
    long count = 0;
    double sum = 0;
    auto& params = p.getParameters();
    for (int pos = 0; pos < (int) (sr * seconds); pos += block)
    {
        buf.clear();
        signal (buf, pos, sr);
        if (side && channels >= 4)
            signal (buf, pos, sr, 2);
        if (wiggle != nullptr && ! params.isEmpty() && (pos / block) % 8 == 0)
            params[wiggle->nextInt (params.size())]->setValue (wiggle->nextFloat());
        p.processBlock (buf, midi);
        for (int c = 0; c < 2; ++c)
            for (int i = 0; i < block; ++i)
            {
                const double v = buf.getSample (c, i);
                if (! std::isfinite (v)) { ++s.bad; continue; }
                s.peak = juce::jmax (s.peak, std::abs (v));
                sum += v * v;
                ++count;
            }
    }
    s.rms = std::sqrt (sum / juce::jmax (1L, count));
    return s;
}

static double db (double x) { return x > 0 ? 20 * std::log10 (x) : -200; }

static void check (const juce::String& name)
{
    const auto file = juce::File::getSpecialLocation (juce::File::userHomeDirectory)
                          .getChildFile ("Library/Audio/Plug-Ins/VST3/Johnny/" + name + ".vst3");
    juce::AudioPluginFormatManager formats;
    formats.addDefaultFormats();
    juce::OwnedArray<juce::PluginDescription> types;
    for (auto* f : formats.getFormats())
        if (f->getName() == "VST3")
            f->findAllTypesForFile (types, file.getFullPathName());
    if (types.size() != 1) { fail ("scan found " + juce::String (types.size()) + " plugins"); return; }
    std::printf ("%s by %s, %s\n", types[0]->name.toRawUTF8(), types[0]->manufacturerName.toRawUTF8(), types[0]->category.toRawUTF8());
    if (types[0]->manufacturerName != "Johnny") fail ("vendor");

    auto make = [&] (double sr, int block)
    {
        juce::String error;
        auto p = formats.createPluginInstance (*types[0], sr, block, error);
        if (p == nullptr) { fail ("instantiate: " + error); return p; }
        p->enableAllBuses();
        p->prepareToPlay (sr, block);
        return p;
    };

    // Rates, blocks, and random settings with automation running.
    juce::Random rng (1);
    double worst = 0;
    for (double sr : { 44100.0, 48000.0, 88200.0, 96000.0, 192000.0 })
        for (int block : { 32, 64, 128, 512, 1024, 4096 })
        {
            auto p = make (sr, block);
            if (p == nullptr) return;
            auto clean = run (*p, sr, block, 0.5);
            if (clean.bad) fail ("non-finite at defaults, " + juce::String (sr) + "/" + juce::String (block));
            auto wild = run (*p, sr, block, 1.0, &rng);
            if (wild.bad) fail ("non-finite while automating, " + juce::String (sr) + "/" + juce::String (block));
            worst = juce::jmax (worst, wild.peak);
            p->releaseResources();
        }

    // Random patches, long enough for feedback to build.
    for (int i = 0; i < 12; ++i)
    {
        auto p = make (48000, 256);
        for (auto* q : p->getParameters()) q->setValue (rng.nextFloat());
        auto s = run (*p, 48000, 256, 4.0);
        if (s.bad) fail ("non-finite on random patch " + juce::String (i));
        worst = juce::jmax (worst, s.peak);
    }

    auto p = make (48000, 256);
    auto dflt = run (*p, 48000, 256, 2.0);
    std::printf ("  %d params, latency %d, stereo in/out %d/%d, defaults rms %.1f peak %.1f dB, worst random peak %.1f dB\n",
                 p->getParameters().size(), p->getLatencySamples(), p->getTotalNumInputChannels(), p->getTotalNumOutputChannels(),
                 db (dflt.rms), db (dflt.peak), db (worst));
    if (dflt.rms < 1e-4) fail ("silent at defaults");

    // Parameter text both ways, and every value reachable without trouble.
    for (auto* q : p->getParameters())
        for (float v : { 0.0f, 0.37f, 1.0f })
        {
            q->setValue (v);
            const auto text = q->getCurrentValueAsText();
            if (text.isEmpty()) fail (q->getName (50) + " has no text");
            const float back = q->getValueForText (text);
            if (std::abs (back - v) > 0.06f && q->getNumSteps() > 256) fail (q->getName (50) + " text " + text + " -> " + juce::String (back));
        }

    // State: set odd values, save, load into a fresh one, compare.
    for (auto* q : p->getParameters()) q->setValue (rng.nextFloat());
    run (*p, 48000, 256, 0.1); // (a VST3 receives parameter changes with the next block)
    juce::MemoryBlock state;
    p->getStateInformation (state);
    auto q = make (48000, 256);
    q->setStateInformation (state.getData(), (int) state.getSize());
    run (*q, 48000, 256, 0.1);
    auto& a = p->getParameters();
    auto& b = q->getParameters();
    for (int i = 0; i < a.size(); ++i)
        // (A stepped parameter snaps to its nearest step, a host's copy of the value doesn't.)
        if (std::abs (a[i]->getValue() - b[i]->getValue()) > juce::jmax (1e-3f, 1.0f / (float) a[i]->getNumSteps()))
            fail ("state lost " + a[i]->getName (50));

    // The sidechain really does something, and only when it has sound.
    if (p->getBusCount (true) > 1)
    {
        auto x = make (48000, 256);
        auto dry = run (*x, 48000, 256, 2.0, nullptr, false);
        auto y = make (48000, 256);
        auto ducked = run (*y, 48000, 256, 2.0, nullptr, true);
        std::printf ("  sidechain off rms %.1f, on rms %.1f dB\n", db (dry.rms), db (ducked.rms));
        if (ducked.rms >= dry.rms * 0.99) fail ("sidechain has no effect");
    }

    // Editor, off screen.
    if (! p->hasEditor()) fail ("no editor");
    else
    {
        std::unique_ptr<juce::AudioProcessorEditor> ed (p->createEditor());
        if (ed == nullptr) fail ("createEditor");
        else
        {
            ed->setOpaque (true);
            ed->addToDesktop (0);
            ed->setTopLeftPosition (-4000, -4000);
            ed->setVisible (true);
            juce::MessageManager::getInstance()->runDispatchLoopUntil (800);
            std::printf ("  editor %d x %d\n", ed->getWidth(), ed->getHeight());
            ed.reset();
            juce::MessageManager::getInstance()->runDispatchLoopUntil (100);
        }
    }
}

struct App : juce::JUCEApplicationBase
{
    const juce::String getApplicationName() override { return "vstcheck"; }
    const juce::String getApplicationVersion() override { return "1"; }
    bool moreThanOneInstanceAllowed() override { return true; }
    void anotherInstanceStarted (const juce::String&) override {}
    void suspended() override {}
    void resumed() override {}
    void shutdown() override {}
    void systemRequestedQuit() override { quit(); }
    void unhandledException (const std::exception*, const juce::String&, int) override {}
    void initialise (const juce::String&) override
    {
        for (auto& name : getCommandLineParameterArray())
            check (name);
        std::printf (failures ? "  %d FAILED\n" : "  ok\n", failures);
        std::fflush (stdout);
        setApplicationReturnValue (failures ? 1 : 0);
        quit();
    }
};

START_JUCE_APPLICATION (App)
