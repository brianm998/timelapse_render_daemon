package RenderRunner;

use strict;

use parent 'ProcessPipe';
use IO::Handle;
use IO::Select; 
use Term::ANSIColor qw(:constants);

# an object that can be used to render an image sequence into a video

sub validate() {
  # we need ffmpeg

  die <<END
ERROR

ffmpeg is not installed

visit https://ffmpeg.org

and install it to use this tool
END
    unless(system("which ffmpeg >/dev/null") == 0);
}


sub new {
  my ($class,
      $ffmpeg_cmd,
      $output_dirname,
      $output_video_filename,
      $group,
      $start_callback,
      $successful_update_callback,
      $finished_callback,
      $group_finished_callback,
     )
      = @_;

  my $self = $class->SUPER::new($ffmpeg_cmd, $group,
				$start_callback,
				$successful_update_callback,
				$finished_callback,
				$group_finished_callback);
  $self->{output_dirname} = $output_dirname;
  $self->{output_video_filename} = $output_video_filename;
  $self->{frame_num} = 0;

  return bless $self, $class;
}

sub read_line($) {
 my ($self) = @_;

 my $ret = undef;
 if(eof($self->{shell_input})) {
   $self->finish();
 } else {
   $/ = ""; # don't use newline for <> XXX explore 'local' for this more
   my $line = sysreadline($self->{shell_input}, 1); # XXX make this non-blocking

   #print "read line of length ", length $line, "\n";
#   chomp $line;
   #print "'$line'\n";
   $/ = "\n";

   my $marker = "^M";
   if($line =~ /$marker\z/) {
     #print "read line '$line'\n";
     # append previous line
     if(exists $self->{partial_line_read}) {
       $line = $self->{partial_line_read}.$line;
       delete($self->{partial_line_read});
     }
     # frame= 1569 fps=0.9 q=-0.0 Lsize=  286475kB time=00:00:52.26 bitrate=44900.6kbits/s speed=0.0292x 
     if ($line =~ /^frame\s*=\s*(\d+)\s+fps\s*=\s*([\d.]+)/) {
       #print "line match\n";
       $self->{frame_num} = $1;
       $self->{fps} = $2;

       my $successful_update_callback = $self->{successful_update_callback};
       &$successful_update_callback($self) if defined $successful_update_callback;
     } else {
       #print "no match\n";
     }
     $ret = 0;
   } else {
     # keep $line for next time around
     $self->{partial_line_read} = $line;
   }
 }
 return $ret;
}

sub finish($) {
  my ($self) = @_;

  if ($self->output_video_exists()) {
    $self->{result} = 1;
  } else {
    # failed, why?
    $self->{result} = 'error';	# XXX expose errors somehow
  }

  $self->SUPER::finish();
}

sub output_video_exists() {
  my ($self) = @_;

  return -e $self->full_output_video_filename();
}

sub full_output_video_filename() {
  my ($self) = @_;

  return "$self->{output_dirname}/$self->{output_video_filename}";
}

use Symbol qw(qualify_to_ref);


sub sysreadline(*;$) {
  my($handle, $timeout) = @_;
  $handle = qualify_to_ref($handle, caller( ));
  my $infinitely_patient = (@_ == 1 || $timeout < 0);
  my $start_time = time( );
  my $selector = IO::Select->new( );
  $selector->add($handle);
  my $line = "";
  my $marker = $/;
  until ($line =~ /$marker\z/) {
    unless ($infinitely_patient) {
      if (time() > ($start_time + $timeout)) {
	#print "bailing out\n";
	return $line;
      }
    }
    # sleep only 1 second before checking again next SLEEP unless $selector->can_read(1.0);

    my $done = 0;
    while (!$done && $selector->can_read(0.0)) {
      my $was_blocking = $handle->blocking(0);
      while (!$done && sysread($handle, my $nextbyte, 1)) {
	$line .= $nextbyte;
	$done = 1 if ($nextbyte eq $/);
      }
      $handle->blocking($was_blocking);
      # if incomplete line, keep trying next SLEEP unless at_eol($line);
    }
  }
  #print "returning full line\n";
  return $line;
}

1;
